#pragma once

#include <JuceHeader.h>

//==============================================================================
/** Presets on disk: Presets/<Bank>/<Preset>.pqrack. Banks are plain folders, so they can also
    be organised in Explorer. */
class PresetLibrary : public juce::ChangeBroadcaster
{
public:
    explicit PresetLibrary (juce::File rootFolder);

    static constexpr const char* extension = ".pqrack";

    const juce::File& getRoot() const noexcept { return root; }

    juce::StringArray getBanks() const;
    juce::File getBankFolder (const juce::String& bank) const { return root.getChildFile (bank); }
    juce::Array<juce::File> getPresets (const juce::String& bank) const;
    static juce::String bankOf (const juce::File& preset) { return preset.getParentDirectory().getFileName(); }

    juce::String getCurrentBank() const { return currentBank; }
    int getCurrentBankIndex() const { return getBanks().indexOf (currentBank); }
    void setCurrentBank (const juce::String& bank);

    /** Each returns an error message, or an empty string on success. */
    juce::String createBank (const juce::String& name);
    juce::String renameBank (const juce::String& bank, const juce::String& newName);
    juce::String deleteBank (const juce::String& bank);           // moves the folder to the recycle bin
    juce::String movePreset (const juce::File& preset, const juce::String& toBank, juce::File& newLocation);

    /** Re-reads the folders (e.g. after a save) and notifies listeners. */
    void refresh() { sendChangeMessage(); }

private:
    juce::File root;
    juce::String currentBank;

    void migrateLoosePresets();
    static juce::String cleanName (const juce::String&);
};
