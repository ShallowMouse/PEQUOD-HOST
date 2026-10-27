#pragma once

#include <JuceHeader.h>
#include "PequodEngineApi.h"

//==============================================================================
/** PEQUOD's own plugins, run natively from the built-in engine: no VST3 files,
    no scanning, panels drawn by the host. They appear in the plugin list with
    the format name "PEQUOD" and the same names, vendors and categories as the
    VST3 versions (whose saved settings they load). */
namespace PequodNative
{
    inline constexpr const char* formatName = "PEQUOD";
    inline constexpr const char* idPrefix   = "pequod:";

    /** One engine plugin's static information (parsed once from the engine). */
    struct Info
    {
        int index = -1;
        juce::String name, vendor, classId, categories;
        bool instrument = false;
        int meters = 0;
        struct Param { juce::String id, name, unit; float defaultValue = 0; int steps = 0; };
        std::vector<Param> params;
        juce::var panel;    // parsed lazily
    };

    /** All plugins in the engine. */
    const std::vector<Info>& all();
    const Info* find (const juce::String& fileOrIdentifier);
    juce::PluginDescription describe (const Info&);

    /** The panel layout JSON, parsed. */
    const juce::var& panelOf (const Info&);

    /** Adds every engine plugin to the known-plugin list (replacing stale entries). */
    void registerAll (juce::KnownPluginList&);

    //==============================================================================
    class Format : public juce::AudioPluginFormat
    {
    public:
        juce::String getName() const override { return formatName; }
        bool canScanForPlugins() const override { return false; }
        bool isTrivialToScan() const override { return true; }
        void findAllTypesForFile (juce::OwnedArray<juce::PluginDescription>&, const juce::String&) override;
        bool fileMightContainThisPluginType (const juce::String& id) override { return id.startsWith (idPrefix); }
        juce::String getNameOfPluginFromIdentifier (const juce::String& id) override;
        bool pluginNeedsRescanning (const juce::PluginDescription&) override { return false; }
        bool doesPluginStillExist (const juce::PluginDescription& d) override { return find (d.fileOrIdentifier) != nullptr; }
        juce::StringArray searchPathsForPlugins (const juce::FileSearchPath&, bool, bool) override;
        juce::FileSearchPath getDefaultLocationsToSearch() override { return {}; }

    private:
        void createPluginInstance (const juce::PluginDescription&, double, int, PluginCreationCallback) override;
        bool requiresUnblockedMessageThreadDuringCreation (const juce::PluginDescription&) const override { return false; }
    };

    //==============================================================================
    class Instance : public juce::AudioPluginInstance
    {
    public:
        explicit Instance (const Info&);
        ~Instance() override;

        const Info& info() const { return inf; }
        PqHandle* handle() const { return h; }

        /** Meter values since the last call (the maximum of each). */
        int takeMeters (float* out, int max) const { return pq_meters (h, out, max); }
        float normalise (int param, float plain) const { return pq_normalize (h, param, plain); }

        // AudioPluginInstance
        void fillInPluginDescription (juce::PluginDescription& d) const override { d = describe (inf); }
        const juce::String getName() const override { return inf.name; }
        void prepareToPlay (double sampleRate, int blockSize) override;
        void releaseResources() override {}
        void reset() override { pq_reset (h); }
        void processBlock (juce::AudioBuffer<float>&, juce::MidiBuffer&) override;
        using juce::AudioPluginInstance::processBlock;
        double getTailLengthSeconds() const override { return 2.0; }
        bool acceptsMidi() const override { return inf.instrument; }
        bool producesMidi() const override { return false; }
        bool isBusesLayoutSupported (const BusesLayout&) const override;
        juce::AudioProcessorEditor* createEditor() override;
        bool hasEditor() const override { return true; }
        int getNumPrograms() override { return 1; }
        int getCurrentProgram() override { return 0; }
        void setCurrentProgram (int) override {}
        const juce::String getProgramName (int) override { return {}; }
        void changeProgramName (int, const juce::String&) override {}
        void getStateInformation (juce::MemoryBlock&) override;
        void setStateInformation (const void*, int) override;

    private:
        const Info& inf;
        PqHandle* h = nullptr;
        std::vector<float*> ptrs;
        juce::AudioBuffer<float> scratch;

        JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (Instance)
    };
}
