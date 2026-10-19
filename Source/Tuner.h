#pragma once

#include "HostEngine.h"

//==============================================================================
/** Guitar tuner: YIN pitch detection on the input stage's tap, run on the message thread. */
class Tuner : private juce::Timer
{
public:
    explicit Tuner (HostEngine& e) : engine (e) { startTimerHz (20); }
    ~Tuner() override { stopTimer(); }

    struct Reading
    {
        bool  valid = false;
        float frequency = 0.0f;
        int   midiNote = 0;
        float cents = 0.0f;        // -50 .. +50
        juce::String noteName;     // e.g. "E", "F#"
        int   octave = 0;
    };

    const Reading& getReading() const noexcept { return reading; }

    float referenceA4 = 440.0f;

    /** Pure function, also used by the self-test. Returns 0 if no clear pitch. */
    static float detectPitch (const float* x, int n, double sampleRate)
    {
        // YIN, restricted to guitar/bass range (30 Hz .. 1100 Hz)
        const int tauMin = juce::jmax (2, (int) (sampleRate / 1100.0));
        const int tauMax = juce::jmin (n / 2, (int) (sampleRate / 30.0));
        const int w = n - tauMax;
        if (w <= 0 || tauMax <= tauMin + 2) return 0.0f;

        std::vector<float> d ((size_t) tauMax + 1, 0.0f);
        for (int tau = 1; tau <= tauMax; ++tau)
        {
            float sum = 0.0f;
            for (int i = 0; i < w; ++i)
            {
                const float diff = x[i] - x[i + tau];
                sum += diff * diff;
            }
            d[(size_t) tau] = sum;
        }

        // Cumulative mean normalised difference
        std::vector<float> cmnd ((size_t) tauMax + 1, 1.0f);
        float running = 0.0f;
        for (int tau = 1; tau <= tauMax; ++tau)
        {
            running += d[(size_t) tau];
            cmnd[(size_t) tau] = running > 0.0f ? d[(size_t) tau] * (float) tau / running : 1.0f;
        }

        int best = -1;
        for (int tau = tauMin; tau < tauMax; ++tau)
        {
            if (cmnd[(size_t) tau] < 0.15f)
            {
                while (tau + 1 < tauMax && cmnd[(size_t) tau + 1] < cmnd[(size_t) tau]) ++tau;
                best = tau;
                break;
            }
        }
        if (best < 0) return 0.0f;

        // Parabolic interpolation around the minimum
        float betterTau = (float) best;
        if (best > 1 && best < tauMax)
        {
            const float a = cmnd[(size_t) best - 1], b = cmnd[(size_t) best], c = cmnd[(size_t) best + 1];
            const float denom = a - 2.0f * b + c;
            if (std::abs (denom) > 1.0e-9f)
                betterTau += 0.5f * (a - c) / denom;
        }
        return (float) (sampleRate / betterTau);
    }

private:
    HostEngine& engine;
    Reading reading;
    std::vector<float> scratch, decimated;
    float history[5] {};
    int historyPos = 0, silentFrames = 0;

    void timerCallback() override
    {
        auto* stage = engine.getInputStage();
        double sr = 48000.0;
        if (auto* dev = engine.deviceManager.getCurrentAudioDevice())
            sr = dev->getCurrentSampleRate();

        // Decimate to ~24 kHz to keep the analysis cheap while staying within ~0.5 cent up the neck.
        const int factor = juce::jmax (1, juce::roundToInt (sr / 24000.0));
        const int outN = 2048;                       // ~85 ms at 24 kHz
        const int inN = outN * factor;

        scratch.resize ((size_t) inN);
        decimated.resize ((size_t) outN);

        const int end = stage->tapWritePos.load (std::memory_order_acquire);
        const int mask = GainMeterProcessor::tapSize - 1;
        for (int i = 0; i < inN; ++i)
            scratch[(size_t) i] = stage->tap[(size_t) ((end - inN + i) & mask)];

        float energy = 0.0f;
        for (int i = 0; i < outN; ++i)
        {
            float s = 0.0f;
            for (int k = 0; k < factor; ++k) s += scratch[(size_t) (i * factor + k)];
            s /= (float) factor;
            decimated[(size_t) i] = s;
            energy += s * s;
        }

        const float rms = std::sqrt (energy / (float) outN);
        float f = rms > 0.003f ? detectPitch (decimated.data(), outN, sr / factor) : 0.0f;

        if (f <= 0.0f)
        {
            // Hold the last note briefly so the display doesn't flicker as a string decays.
            if (++silentFrames > 6) reading.valid = false;
            return;
        }
        silentFrames = 0;

        // Median of the last five estimates rejects octave blips.
        history[historyPos] = f;
        historyPos = (historyPos + 1) % 5;
        float sorted[5];
        std::copy (std::begin (history), std::end (history), sorted);
        std::sort (std::begin (sorted), std::end (sorted));
        const float med = sorted[2] > 0.0f ? sorted[2] : f;
        f = std::abs (f - med) / med < 0.03f ? f : med;

        const float noteF = 69.0f + 12.0f * std::log2 (f / referenceA4);
        const int note = juce::roundToInt (noteF);
        const float cents = (noteF - (float) note) * 100.0f;

        // Ease the needle
        reading.cents = reading.valid && reading.midiNote == note ? reading.cents * 0.6f + cents * 0.4f : cents;
        reading.valid = true;
        reading.frequency = f;
        reading.midiNote = note;
        reading.noteName = juce::MidiMessage::getMidiNoteName (note, true, false, 4);
        reading.octave = note / 12 - 1;
    }
};
