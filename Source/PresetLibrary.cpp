#include "PresetLibrary.h"

PresetLibrary::PresetLibrary (juce::File rootFolder) : root (std::move (rootFolder))
{
    root.createDirectory();
    migrateLoosePresets();

    if (getBanks().isEmpty())
        root.getChildFile ("Default").createDirectory();

    currentBank = getBanks()[0];
}

void PresetLibrary::migrateLoosePresets()
{
    // Presets saved before banks existed live directly in the root: move them into "Default".
    auto loose = root.findChildFiles (juce::File::findFiles, false, juce::String ("*") + extension);
    if (loose.isEmpty()) return;

    auto dest = root.getChildFile ("Default");
    dest.createDirectory();
    for (auto& f : loose)
    {
        auto target = dest.getChildFile (f.getFileName());
        if (target.exists())
            target = dest.getNonexistentChildFile (f.getFileNameWithoutExtension(), extension, false);
        f.moveFileTo (target);
    }
}

juce::String PresetLibrary::cleanName (const juce::String& s)
{
    return juce::File::createLegalFileName (s.trim()).trim();
}

juce::StringArray PresetLibrary::getBanks() const
{
    juce::StringArray banks;
    for (auto& d : root.findChildFiles (juce::File::findDirectories, false))
        if (! d.isHidden())
            banks.add (d.getFileName());
    banks.sortNatural();
    return banks;
}

juce::Array<juce::File> PresetLibrary::getPresets (const juce::String& bank) const
{
    if (bank.isEmpty()) return {};
    auto files = getBankFolder (bank).findChildFiles (juce::File::findFiles, false, juce::String ("*") + extension);
    std::sort (files.begin(), files.end(), [] (const juce::File& a, const juce::File& b)
    {
        return a.getFileNameWithoutExtension().compareNatural (b.getFileNameWithoutExtension()) < 0;
    });
    return files;
}

void PresetLibrary::setCurrentBank (const juce::String& bank)
{
    if (bank != currentBank && getBanks().contains (bank))
    {
        currentBank = bank;
        sendChangeMessage();
    }
}

juce::String PresetLibrary::createBank (const juce::String& name)
{
    auto n = cleanName (name);
    if (n.isEmpty()) return "Please enter a bank name";
    auto dir = getBankFolder (n);
    if (dir.exists()) return "A bank called \"" + n + "\" already exists";
    if (! dir.createDirectory()) return "Couldn't create " + dir.getFullPathName();
    currentBank = n;
    sendChangeMessage();
    return {};
}

juce::String PresetLibrary::renameBank (const juce::String& bank, const juce::String& newName)
{
    auto n = cleanName (newName);
    if (n.isEmpty()) return "Please enter a bank name";
    if (n == bank) return {};
    auto target = getBankFolder (n);
    if (target.exists()) return "A bank called \"" + n + "\" already exists";
    if (! getBankFolder (bank).moveFileTo (target)) return "Couldn't rename the bank";
    if (currentBank == bank) currentBank = n;
    sendChangeMessage();
    return {};
}

juce::String PresetLibrary::deleteBank (const juce::String& bank)
{
    if (getBanks().size() <= 1) return "You need at least one bank";
    if (! getBankFolder (bank).moveToTrash()) return "Couldn't delete the bank";
    if (currentBank == bank) currentBank = getBanks()[0];
    sendChangeMessage();
    return {};
}

juce::String PresetLibrary::movePreset (const juce::File& preset, const juce::String& toBank, juce::File& newLocation)
{
    auto dir = getBankFolder (toBank);
    if (! dir.isDirectory()) return "No bank called " + toBank;
    auto target = dir.getChildFile (preset.getFileName());
    if (target.exists()) target = dir.getNonexistentChildFile (preset.getFileNameWithoutExtension(), extension, false);
    if (! preset.moveFileTo (target)) return "Couldn't move the preset";
    newLocation = target;
    sendChangeMessage();
    return {};
}
