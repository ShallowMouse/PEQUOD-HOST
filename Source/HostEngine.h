#pragma once

#include <JuceHeader.h>
#include <atomic>
#include <memory>
#include <vector>
#include "RackProcessor.h"
#include "Transport.h"

//==============================================================================
/** Input conditioning + gain + peak metering. Used for the master IN and OUT stages. */
class GainMeterProcessor : public juce::AudioProcessor
{
public:
    enum class InputMode { Stereo = 0, Left, Right, Sum };

    GainMeterProcessor()
        : AudioProcessor (BusesProperties().withInput  ("In",  juce::AudioChannelSet::stereo(), true)
                                           .withOutput ("Out", juce::AudioChannelSet::stereo(), true)) {}

    std::atomic<float> gainDb { 0.0f };
    std::atomic<int>   inputMode { (int) InputMode::Stereo };
    std::atomic<float> peakL { 0.0f }, peakR { 0.0f };
    std::atomic<bool>  muted { false };

    /** Mono tap of the conditioned (pre-gain) signal, read by the tuner. */
    static constexpr int tapSize = 16384;
    std::vector<float> tap = std::vector<float> ((size_t) tapSize, 0.0f);
    std::atomic<int>   tapWritePos { 0 };
    bool tapEnabled = false;

    void prepareToPlay (double, int) override { smoothed.reset (getSampleRate() > 0 ? getSampleRate() : 48000.0, 0.02); smoothed.setCurrentAndTargetValue (juce::Decibels::decibelsToGain (gainDb.load())); }
    void releaseResources() override {}

    using AudioProcessor::processBlock;

    void processBlock (juce::AudioBuffer<float>& buffer, juce::MidiBuffer&) override
    {
        const int n = buffer.getNumSamples();
        if (buffer.getNumChannels() < 2) return;

        auto* l = buffer.getWritePointer (0);
        auto* r = buffer.getWritePointer (1);

        switch ((InputMode) inputMode.load())
        {
            case InputMode::Left:  juce::FloatVectorOperations::copy (r, l, n); break;
            case InputMode::Right: juce::FloatVectorOperations::copy (l, r, n); break;
            case InputMode::Sum:
                for (int i = 0; i < n; ++i) { auto s = 0.5f * (l[i] + r[i]); l[i] = s; r[i] = s; }
                break;
            case InputMode::Stereo: default: break;
        }

        if (tapEnabled)
        {
            int w = tapWritePos.load (std::memory_order_relaxed);
            for (int i = 0; i < n; ++i)
            {
                tap[(size_t) w] = 0.5f * (l[i] + r[i]);
                w = (w + 1) & (tapSize - 1);
            }
            tapWritePos.store (w, std::memory_order_release);
        }

        smoothed.setTargetValue (muted.load() ? 0.0f : juce::Decibels::decibelsToGain (gainDb.load(), -60.0f));
        float pl = 0.0f, pr = 0.0f;
        for (int i = 0; i < n; ++i)
        {
            auto g = smoothed.getNextValue();
            l[i] *= g; r[i] *= g;
            pl = juce::jmax (pl, std::abs (l[i]));
            pr = juce::jmax (pr, std::abs (r[i]));
        }
        peakL.store (juce::jmax (pl, peakL.load() * 0.92f));
        peakR.store (juce::jmax (pr, peakR.load() * 0.92f));
    }

    const juce::String getName() const override             { return "GainMeter"; }
    bool acceptsMidi() const override                        { return false; }
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

private:
    juce::SmoothedValue<float> smoothed;
};

//==============================================================================
/** Receives every parameter change of every hosted plugin (from any thread). */
struct ParamObserver
{
    virtual ~ParamObserver() = default;
    virtual void pluginParameterChanged (int slotUid, int paramIndex, float value) = 0;
    virtual void pluginParameterGesture (int slotUid, int paramIndex) = 0;
};

//==============================================================================
struct PluginSlot
{
    int uid = 0;
    juce::String stableId;                  // saved in presets; survives reordering and reloading
    std::unique_ptr<juce::AudioPluginInstance> plugin;
    juce::PluginDescription description;
    std::atomic<bool> bypassed { false };
    std::atomic<float> cpuLoad { 0.0f };    // share of one block's time this plugin takes (0..1+)

    juce::AudioPluginInstance* instance() const { return plugin.get(); }

    struct Listener;
    std::unique_ptr<Listener> listener;
};

/** Split settings shared with the audio thread. */
struct SplitState
{
    std::atomic<float> mix { 0.5f };
};

struct RackItem
{
    enum class Type { plugin, split };

    int uid = 0;
    juce::String stableId;                  // splits: saved in presets, used by mappings
    Type type = Type::plugin;

    std::shared_ptr<PluginSlot> plugin;                    // Type::plugin
    std::vector<std::shared_ptr<PluginSlot>> lanes[2];     // Type::split
    std::shared_ptr<SplitState> state;                     // Type::split
    float mix = 0.5f;
};

/** Where a plugin sits / should go. lane == -1 means the top level of the rack. */
struct RackLocation
{
    int itemIndex = -1;  // top-level index (insert position when lane == -1)
    int lane = -1;       // 0 = A, 1 = B
    int laneIndex = 0;   // insert position inside the lane
};

//==============================================================================
class HostEngine : public juce::ChangeBroadcaster,
                   private juce::ChangeListener,
                   private juce::Timer
{
public:
    HostEngine();
    ~HostEngine() override;

    //== Devices
    juce::AudioDeviceManager deviceManager;
    juce::AudioPluginFormatManager formatManager;
    juce::KnownPluginList knownPlugins;
    juce::AudioProcessorPlayer player;

    void initialiseAudio (const juce::XmlElement* savedDeviceState);

    //== Rack
    const std::vector<RackItem>& getItems() const noexcept { return items; }

    /** Adds a plugin; returns the new slot uid or 0 on failure (errorOut filled). */
    int addPlugin (const juce::PluginDescription&, RackLocation where, juce::String& errorOut);
    int addSplit (int itemIndex);
    void removeSlot (int slotUid);
    void removeItem (int itemUid);
    void moveSlot (int slotUid, RackLocation where);
    void moveItem (int itemUid, int newIndex);
    void clearRack();

    void setBypassed (int slotUid, bool shouldBypass);
    void setSplitMix (int itemUid, float mix);

    std::shared_ptr<PluginSlot> findSlot (int slotUid) const;
    /** Runtime uid of the slot or split with this saved ID, 0 if it isn't in the rack. */
    int findUidByStableId (const juce::String& stableId, bool isSplit) const;
    juce::String getStableId (int uid, bool isSplit) const;
    static juce::String newStableId() { return juce::Uuid().toString().substring (0, 12); }
    RackItem* findItem (int itemUid);
    bool locateSlot (int slotUid, RackLocation& out) const;

    /** Every plugin in rack order (lane A before lane B). Used for 1-based OSC numbering. */
    std::vector<std::shared_ptr<PluginSlot>> getFlatSlots() const;
    int getFlatIndex (int slotUid) const; // 1-based, 0 if not found
    /** Splits in rack order (1-based OSC numbering). */
    std::vector<RackItem*> getSplits();
    int getSplitNumber (int itemUid) const;

    //== Master
    Transport transport;                       // tempo (saved with the preset) + click
    GainMeterProcessor* getInputStage() const  { return inputStage; }
    GainMeterProcessor* getOutputStage() const { return outputStage; }

    //== Presets
    std::unique_ptr<juce::XmlElement> createRackXml() const;
    bool loadRackXml (const juce::XmlElement&, juce::StringArray& errors);

    juce::String currentPresetName { "Init" };

    //== Parameter observation (OSC feedback, learn)
    void setParamObserver (ParamObserver* o) { observer = o; }

    std::function<void (int slotUid)> onSlotAboutToBeRemoved;

    /** Rebuilds the audio thread's render plan from the rack (call after any structural change). */
    void rewire();

    RackProcessor& getRack() noexcept { return *rack; }

    //== Multi-core
    /** How many cores the serial chain is spread over (1 = no pipeline, no extra latency). */
    void setPipelineStages (int stages);
    int getPipelineStages() const noexcept { return pipelineStages; }
    int getMaxPipelineStages() const noexcept;
    /** Extra latency added by the pipeline, in samples. */
    int getPipelineLatencySamples() const noexcept;
    /** Combined CPU share of a rack item (both lanes for a split). */
    float getItemLoad (const RackItem&) const;
    /** Plugin CPU on the busiest core, as a share of the block time. */
    float getBusiestCoreLoad() const;

    /** Increments on every rack change (used to show the preset as modified). */
    int getChangeCounter() const noexcept { return changeCounter; }
    void changed() { ++changeCounter; sendChangeMessage(); }
    /** The preset has unsaved edits (that aren't rack changes, e.g. this-preset footswitch settings). */
    void markModified() noexcept { ++changeCounter; }

    /** Call after structural changes: the pipeline is re-balanced once CPU use has been measured. */
    void scheduleRebalance() { rebalanceAt = juce::Time::getMillisecondCounterHiRes() + 3000.0; }

private:
    int changeCounter = 0;
    std::unique_ptr<GainMeterProcessor> inputStageOwned, outputStageOwned;
    GainMeterProcessor* inputStage = nullptr;
    GainMeterProcessor* outputStage = nullptr;
    std::unique_ptr<RackProcessor> rack;

    int pipelineStages = 1;
    double rebalanceAt = 0;
    int lastPlanLatency = 0;

    std::vector<RackItem> items;
    int nextUid = 1;
    ParamObserver* observer = nullptr;

    std::shared_ptr<PluginSlot> createSlot (const juce::PluginDescription&, juce::String& error);
    void destroySlot (const std::shared_ptr<PluginSlot>&);
    std::shared_ptr<PluginSlot> detachSlot (int slotUid);
    void insertSlot (std::shared_ptr<PluginSlot>, RackLocation where);
    void attachListener (PluginSlot&);

    std::unique_ptr<RenderPlugin> makeRenderPlugin (PluginSlot&, int maxBlock) const;
    int computeChainLatency() const;
    std::vector<int> computePartition (int numStages) const;
    float busiestStageLoad (const std::vector<int>&) const;
    std::vector<int> currentPartition;

    void changeListenerCallback (juce::ChangeBroadcaster*) override;
    void timerCallback() override;

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (HostEngine)
};
