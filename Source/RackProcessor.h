#pragma once

#include "WorkerPool.h"

class GainMeterProcessor;
class Transport;
struct PluginSlot;

//==============================================================================
/** Everything the audio thread needs for one hosted plugin. */
struct RenderPlugin
{
    PluginSlot* slot = nullptr;
    juce::AudioPluginInstance* inst = nullptr;
    int numIn = 2, numOut = 2, numScratch = 2;
    bool wantsMidi = false;
    juce::AudioBuffer<float> scratch;
    juce::MidiBuffer midi;
};

/** A simple stereo delay used to line up the two lanes of a split. */
struct LaneDelay
{
    int delay = 0;
    int writePos = 0;
    juce::AudioBuffer<float> buffer;

    void prepare (int delaySamples, int maxBlock)
    {
        delay = delaySamples;
        buffer.setSize (2, juce::jmax (1, delay + maxBlock + 1));
        buffer.clear();
        writePos = 0;
    }

    void process (juce::AudioBuffer<float>& io, int n) noexcept
    {
        if (delay <= 0) return;
        const int size = buffer.getNumSamples();
        for (int ch = 0; ch < 2; ++ch)
        {
            auto* d = buffer.getWritePointer (ch);
            auto* x = io.getWritePointer (ch);
            int w = writePos;
            for (int i = 0; i < n; ++i)
            {
                d[w] = x[i];
                int r = w - delay;
                if (r < 0) r += size;
                x[i] = d[r];
                if (++w == size) w = 0;
            }
        }
        writePos = (writePos + n) % size;
    }
};

struct RenderLane
{
    std::vector<std::unique_ptr<RenderPlugin>> plugins;
    int latency = 0;
    LaneDelay align;
    juce::AudioBuffer<float> buffer;
    juce::MidiBuffer midi;
};

struct SplitState;

struct RenderItem
{
    bool isSplit = false;
    std::unique_ptr<RenderPlugin> plugin;   // !isSplit
    RenderLane lanes[2];                    // isSplit
    SplitState* split = nullptr;
    juce::SmoothedValue<float> mix;
    int numSamples = 0;                     // set per block
    juce::MidiBuffer* midiIn = nullptr;     // set per block
    class RackProcessor* owner = nullptr;
};

struct RenderStage
{
    std::vector<RenderItem*> items;
};

/** Immutable-shape snapshot of the rack, built on the message thread and swapped in. */
struct RenderPlan
{
    std::vector<std::unique_ptr<RenderItem>> items;
    std::vector<RenderStage> stages;         // 1 = no pipeline
    int maxBlock = 0;
    double sampleRate = 0;
    int pluginLatency = 0;                   // summed latency of the chain (longest lane in splits)
    std::vector<PluginSlot*> instruments;    // VSTis in the rack: while one is on, the guitar is muted

    // Pipeline buffers: one per stage, rotated every block.
    std::vector<juce::AudioBuffer<float>> stageAudio;
    std::vector<juce::MidiBuffer> stageMidi;
    std::vector<int> stageCount, ring;
};

//==============================================================================
/** The audio processor the device plays: input stage -> rack (multi-core) -> output stage. */
class RackProcessor : public juce::AudioProcessor
{
public:
    RackProcessor (GainMeterProcessor& in, GainMeterProcessor& out);
    ~RackProcessor() override;

    /** Swaps in a new plan; returns the old one so the caller can delete it off the audio thread. */
    std::unique_ptr<RenderPlan> swapPlan (std::unique_ptr<RenderPlan>);

    /** Fades the rack's output out (or back in) over a few ms. Used to switch presets without a click
        or a burst of dry guitar. waitForSilence returns once the fade has reached zero (or audio isn't running). */
    void fadeOut()  { fadeTarget = 0.0f; }
    void fadeIn()   { fadeTarget = 1.0f; }
    bool waitForSilence (int timeoutMs = 60) const;

    /** Notes from the on-screen keyboard (merged with the MIDI inputs). */
    juce::MidiKeyboardState keyboardState;

    /** Mute the guitar while an instrument plugin is on (so only the instrument is heard). */
    std::atomic<bool> muteGuitarForInstruments { true };
    /** True while the guitar is muted because an instrument is on (for the UI). */
    bool isGuitarMutedForInstrument() const noexcept { return guitarMuted.load(); }

    /** The master tempo every plugin sees (and the click). */
    Transport* transport = nullptr;

    double getCurrentSampleRate() const noexcept { return currentRate.load(); }
    int getCurrentBlockSize() const noexcept     { return currentBlock.load(); }
    int getNumWorkerThreads() const noexcept     { return pool.getNumThreads(); }
    WorkerPool& getPool() noexcept               { return pool; }

    /** Set by the engine when the device restarts with new settings and the plan needs rebuilding. */
    std::function<void()> onNeedsNewPlan;

    // Prepares a plugin for the current device settings (safe from the message thread).
    void preparePlugin (juce::AudioPluginInstance&) const;

    //== AudioProcessor
    void prepareToPlay (double sampleRate, int blockSize) override;
    void releaseResources() override {}
    void processBlock (juce::AudioBuffer<float>&, juce::MidiBuffer&) override;
    using AudioProcessor::processBlock;
    bool isBusesLayoutSupported (const BusesLayout&) const override;

    const juce::String getName() const override             { return "PEQUOD Rack"; }
    bool acceptsMidi() const override                        { return true; }
    bool producesMidi() const override                       { return false; }
    double getTailLengthSeconds() const override             { return 0.0; }
    juce::AudioProcessorEditor* createEditor() override      { return nullptr; }
    bool hasEditor() const override                          { return false; }
    int getNumPrograms() override                            { return 1; }
    int getCurrentProgram() override                         { return 0; }
    void setCurrentProgram (int) override                    {}
    const juce::String getProgramName (int) override         { return {}; }
    void changeProgramName (int, const juce::String&) override {}
    void getStateInformation (juce::MemoryBlock&) override   {}
    void setStateInformation (const void*, int) override     {}

    // Used by the render code
    void processPlugin (RenderPlugin&, juce::AudioBuffer<float>& stereo, const juce::MidiBuffer& midiIn, int n) noexcept;
    void processItem (RenderItem&, juce::AudioBuffer<float>& stereo, juce::MidiBuffer& midi, int n) noexcept;
    void processStage (RenderStage&, juce::AudioBuffer<float>& stereo, juce::MidiBuffer& midi, int n) noexcept;

private:
    GainMeterProcessor& inputStage;
    GainMeterProcessor& outputStage;
    WorkerPool pool;

    juce::CriticalSection planLock;
    std::unique_ptr<RenderPlan> plan;

    std::atomic<double> currentRate { 48000.0 };
    std::atomic<int> currentBlock { 512 };
    juce::AudioBuffer<float> io;
    juce::MidiBuffer emptyMidi;
    std::atomic<float> fadeTarget { 1.0f }, fadeNow { 1.0f };
    float fadeGain = 1.0f;
    float guitarGain = 1.0f;
    std::atomic<bool> guitarMuted { false };

    void processPlan (RenderPlan&, juce::AudioBuffer<float>& stereo, juce::MidiBuffer& midi, int n) noexcept;

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (RackProcessor)
};
