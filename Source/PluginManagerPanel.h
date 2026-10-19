#pragma once

#include "HostEngine.h"
#include "PequodLookAndFeel.h"

//==============================================================================
/** The PLUGINS window: the VST3 folders to scan (kept between runs) plus the plugin list. */
class PluginManagerPanel : public juce::Component
{
public:
    PluginManagerPanel (HostEngine&, juce::PropertiesFile&, juce::File deadMansPedal);
    ~PluginManagerPanel() override;

    /** Scans the folders in the list straight away (no extra dialog). */
    void scanFolders();

    std::function<void (const juce::String&)> onStatus;

    void paint (juce::Graphics&) override;
    void resized() override;

private:
    HostEngine& engine;
    juce::PropertiesFile& props;
    juce::AudioPluginFormat* vst3 = nullptr;

    juce::Label foldersCaption, foldersHelp, listCaption;
    juce::FileSearchPathListComponent folders;
    juce::TextButton scanButton { "SCAN FOLDERS" }, defaultsButton { "RESET TO DEFAULTS" };
    std::unique_ptr<juce::PluginListComponent> pluginList;

    void savePath();
};
