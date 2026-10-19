#pragma once

#include "HostEngine.h"
#include "PequodLookAndFeel.h"

//==============================================================================
/** Left-hand browser: PRESETS / PLUGINS tabs, search, list. Plugins drag into the rack. */
class BrowserPanel : public juce::Component,
                     private juce::ListBoxModel,
                     private juce::ChangeListener
{
public:
    BrowserPanel (HostEngine&, juce::File presetDirectory);
    ~BrowserPanel() override;

    void refresh();
    void showPresets (bool presets);
    void setCurrentPresetFile (const juce::File& f) { currentPreset = f; list.repaint(); }

    std::function<void (const juce::PluginDescription&)> onAddPlugin;
    std::function<void (const juce::File&)> onLoadPreset;
    std::function<void (const juce::File&)> onDeletePreset;

    void paint (juce::Graphics&) override;
    void resized() override;

    /** Drag description prefix for plugins. */
    static juce::String dragIdForPlugin (int knownIndex) { return "plugin:" + juce::String (knownIndex); }

private:
    struct Row
    {
        bool isHeader = false;
        juce::String text, subText;
        int pluginIndex = -1;
        juce::File file;
    };

    HostEngine& engine;
    juce::File presetDir, currentPreset;
    bool presetsTab = false;

    juce::TextButton presetsTabButton { "PRESETS" }, pluginsTabButton { "PLUGINS" };
    juce::TextEditor search;
    juce::ListBox list { "browser", this };
    std::vector<Row> rows;

    int getNumRows() override { return (int) rows.size(); }
    void paintListBoxItem (int row, juce::Graphics&, int w, int h, bool selected) override;
    void listBoxItemDoubleClicked (int row, const juce::MouseEvent&) override;
    void listBoxItemClicked (int row, const juce::MouseEvent&) override;
    void activateRow (int row);
    juce::var getDragSourceDescription (const juce::SparseSet<int>& rows) override;
    juce::String getTooltipForRow (int row) override;
    void changeListenerCallback (juce::ChangeBroadcaster*) override { refresh(); }
};
