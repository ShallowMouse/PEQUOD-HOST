#pragma once

#include "HostEngine.h"
#include "PequodLookAndFeel.h"
#include "PresetLibrary.h"
#include "PluginTags.h"

//==============================================================================
/** Left-hand browser: PRESETS / PLUGINS tabs, search, list. Plugins drag into the rack. */
class BrowserPanel : public juce::Component,
                     private juce::ListBoxModel,
                     private juce::ChangeListener
{
public:
    BrowserPanel (HostEngine&, PresetLibrary&);
    ~BrowserPanel() override;

    void refresh();
    void showPresets (bool presets);
    void setCurrentPresetFile (const juce::File& f) { currentPreset = f; list.repaint(); }

    /** How the PLUGINS tab is laid out: by function (amps, drives, delays...) or by collection. */
    enum class Grouping { function = 0, collection = 1 };
    void setGrouping (Grouping);
    Grouping getGrouping() const { return grouping; }
    std::function<void (Grouping)> onGroupingChanged;

    std::function<void (const juce::PluginDescription&)> onAddPlugin;
    std::function<void (const juce::File&)> onLoadPreset;
    std::function<void (const juce::File&)> onDeletePreset;
    std::function<void()> onNewBank;
    std::function<void (const juce::String& bank)> onRenameBank, onDeleteBank;
    std::function<void (const juce::File&, const juce::String& bank)> onMovePreset;

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
        juce::String bank;          // bank header rows
        juce::Colour colour;        // plugin rows and group headers: the function's colour
    };

    HostEngine& engine;
    PresetLibrary& library;
    juce::File currentPreset;
    bool presetsTab = false;
    Grouping grouping = Grouping::function;
    int hiddenDuplicates = 0;

    juce::TextButton presetsTabButton { "PRESETS" }, pluginsTabButton { "PLUGINS" };
    juce::TextEditor search;
    juce::TextButton newBankButton { "+ BANK" };
    juce::TextButton byFunctionButton { "BY FUNCTION" }, byCollectionButton { "BY COLLECTION" };
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
    void showBankMenu (const juce::String& bank);
    void showPresetMenu (const juce::File& preset);
};
