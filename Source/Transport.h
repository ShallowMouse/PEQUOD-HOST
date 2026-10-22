#pragma once

#include <JuceHeader.h>

//==============================================================================
/** The master tempo. Every hosted plugin sees it as a running transport, so tempo-synced
    delays, tremolos and LFOs follow it. Also tap tempo and an optional click. */
class Transport : public juce::AudioPlayHead
{
public:
    std::atomic<double> bpm { 120.0 };
    std::atomic<int> beatsPerBar { 4 };
    std::atomic<bool> clickOn { false };
    std::atomic<float> clickGainDb { -12.0f };

    static constexpr double minBpm = 30.0, maxBpm = 300.0;

    void setBpm (double b)
    {
        bpm = juce::jlimit (minBpm, maxBpm, std::round (b * 10.0) / 10.0);
    }

    /** Message thread. Several taps set the tempo from their spacing; each tap starts a beat. */
    void tap()
    {
        const double now = juce::Time::getMillisecondCounterHiRes();
        if (! taps.empty() && now - taps.back() > 2000.0) taps.clear();
        taps.push_back (now);
        if (taps.size() > 5) taps.erase (taps.begin());
        if (taps.size() >= 2)
        {
            const double avg = (taps.back() - taps.front()) / (double) (taps.size() - 1);
            if (avg > 0.0) setBpm (60000.0 / avg);
        }
        resyncRequested = true;
    }
    int tapCount() const { return (int) taps.size(); }

    /** Where the beat is (message thread, for the display / LEDs). */
    double getUiPpq() const { return uiPpq.load(); }
    bool isOnBeatFlash() const
    {
        const double p = getUiPpq();
        return p - std::floor (p) < 0.2;
    }
    int currentBeatInBar() const
    {
        const double p = getUiPpq();
        return (int) std::fmod (std::floor (p), (double) juce::jmax (1, beatsPerBar.load()));
    }

    //== audio thread
    void beginBlock (double sampleRate)
    {
        sr = sampleRate > 0.0 ? sampleRate : 48000.0;
        if (resyncRequested.exchange (false))
            ppq = std::floor (ppq) + 1.0 - 1e-9;          // a tap starts a new beat right now
        blockPpq = ppq;
        blockSamples = samples;
    }

    /** Adds the click (accent on beat 1) and moves the transport on. */
    void endBlock (juce::AudioBuffer<float>& stereo, int n)
    {
        const double perSample = bpm.load() / (60.0 * sr);
        const bool click = clickOn.load();
        const float gain = juce::Decibels::decibelsToGain (clickGainDb.load());
        auto* L = stereo.getWritePointer (0);
        auto* R = stereo.getNumChannels() > 1 ? stereo.getWritePointer (1) : nullptr;

        for (int i = 0; i < n; ++i)
        {
            const double next = ppq + perSample;
            if (std::floor (next) > std::floor (ppq))           // a beat starts in this sample
            {
                const int beat = (int) std::fmod (std::floor (next), (double) juce::jmax (1, beatsPerBar.load()));
                clickFreq = beat == 0 ? 1760.0 : 1320.0;
                clickAmp = beat == 0 ? 1.0f : 0.6f;
                clickPhase = 0.0;
                clickLeft = (int) (0.03 * sr);
            }
            ppq = next;
            if (click && clickLeft > 0)
            {
                const float env = clickAmp * (float) clickLeft / (float) (0.03 * sr);
                const float s = gain * env * (float) std::sin (clickPhase);
                clickPhase += juce::MathConstants<double>::twoPi * clickFreq / sr;
                L[i] += s;
                if (R != nullptr) R[i] += s;
            }
            if (clickLeft > 0) --clickLeft;
        }
        if (ppq > 1.0e7) ppq = std::fmod (ppq, (double) juce::jmax (1, beatsPerBar.load()));
        samples += n;
        uiPpq = ppq;
    }

    juce::Optional<PositionInfo> getPosition() const override
    {
        PositionInfo p;
        const double b = bpm.load();
        const int bars = juce::jmax (1, beatsPerBar.load());
        p.setBpm (b);
        p.setTimeSignature (TimeSignature { bars, 4 });
        p.setPpqPosition (blockPpq);
        p.setPpqPositionOfLastBarStart (std::floor (blockPpq / bars) * bars);
        p.setTimeInSamples (blockSamples);
        p.setTimeInSeconds ((double) blockSamples / sr);
        p.setIsPlaying (true);
        p.setIsRecording (false);
        p.setIsLooping (false);
        return p;
    }

private:
    std::vector<double> taps;
    std::atomic<bool> resyncRequested { false };
    std::atomic<double> uiPpq { 0.0 };
    double sr = 48000.0, ppq = 0.0, blockPpq = 0.0;
    int64_t samples = 0, blockSamples = 0;
    double clickPhase = 0.0, clickFreq = 1320.0;
    float clickAmp = 0.0f;
    int clickLeft = 0;
};
