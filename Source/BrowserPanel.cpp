#include <set>
#include "BrowserPanel.h"
#include "PequodNative.h"

BrowserPanel::BrowserPanel (HostEngine& e, PresetLibrary& lib) : engine (e), library (lib)
{
    newBankButton.setTooltip ("Create a new preset bank");
    newBankButton.onClick = [this] { if (onNewBank) onNewBank(); };
    addChildComponent (newBankButton);

    for (auto* b : { &presetsTabButton, &pluginsTabButton })
    {
        b->setClickingTogglesState (false);
        b->setColour (juce::TextButton::buttonColourId, Pequod::Colours::panel);
        addAndMakeVisible (b);
    }
    presetsTabButton.onClick = [this] { showPresets (true); };

    for (auto* b : { &byFunctionButton, &byCollectionButton })
    {
        b->setClickingTogglesState (false);
        b->setColour (juce::TextButton::buttonColourId, Pequod::Colours::panel);
        addChildComponent (b);
    }
    byFunctionButton.setTooltip ("Group plugins by what they do: amps, drives, delays, reverbs...");
    byCollectionButton.setTooltip ("Group plugins by the collection they come from");
    byFunctionButton.onClick   = [this] { setGrouping (Grouping::function); if (onGroupingChanged) onGroupingChanged (grouping); };
    byCollectionButton.onClick = [this] { setGrouping (Grouping::collection); if (onGroupingChanged) onGroupingChanged (grouping); };
    byFunctionButton.setToggleState (true, juce::dontSendNotification);
    pluginsTabButton.onClick = [this] { showPresets (false); };

    search.setTextToShowWhenEmpty ("Search...", Pequod::Colours::textDim);
    search.setFont (Pequod::body (14.0f));
    search.onTextChange = [this] { refresh(); };
    search.onEscapeKey = [this] { search.clear(); refresh(); };
    addAndMakeVisible (search);

    list.setRowHeight (30);
    list.setColour (juce::ListBox::backgroundColourId, Pequod::Colours::well);
    addAndMakeVisible (list);

    engine.knownPlugins.addChangeListener (this);
    library.addChangeListener (this);
    showPresets (false);
}

BrowserPanel::~BrowserPanel()
{
    library.removeChangeListener (this);
    engine.knownPlugins.removeChangeListener (this);
}

void BrowserPanel::showPresets (bool p)
{
    presetsTab = p;
    presetsTabButton.setToggleState (p, juce::dontSendNotification);
    pluginsTabButton.setToggleState (! p, juce::dontSendNotification);
    newBankButton.setVisible (p);
    byFunctionButton.setVisible (! p);
    byCollectionButton.setVisible (! p);
    resized();
    refresh();
}

void BrowserPanel::setGrouping (Grouping g)
{
    grouping = g;
    byFunctionButton.setToggleState (g == Grouping::function, juce::dontSendNotification);
    byCollectionButton.setToggleState (g == Grouping::collection, juce::dontSendNotification);
    refresh();
    list.scrollToEnsureRowIsOnscreen (0);
}

void BrowserPanel::refresh()
{
    rows.clear();
    const auto filter = search.getText().trim();

    if (presetsTab)
    {
        for (auto& bank : library.getBanks())
        {
            auto presets = library.getPresets (bank);
            juce::Array<juce::File> shown;
            for (auto& f : presets)
                if (filter.isEmpty() || f.getFileNameWithoutExtension().containsIgnoreCase (filter)
                     || bank.containsIgnoreCase (filter))
                    shown.add (f);

            if (filter.isNotEmpty() && shown.isEmpty()) continue;

            rows.push_back ({ true, bank.toUpperCase(), juce::String (presets.size()), -1, {}, bank });
            for (auto& f : shown)
                rows.push_back ({ false, f.getFileNameWithoutExtension(),
                                  juce::String (presets.indexOf (f) + 1).paddedLeft ('0', 2), -1, f, {} });
            if (presets.isEmpty())
                rows.push_back ({ true, "   (empty - SAVE AS into this bank)", {}, -1, {}, {} });
        }

        if (rows.empty())
            rows.push_back ({ true, "No matches", {}, -1, {}, {} });
    }
    else
    {
        auto types = engine.knownPlugins.getTypes();
        const bool byFunction = grouping == Grouping::function;

        struct Entry { int index; juce::String group, function, collection; int order; };
        std::vector<Entry> entries;
        std::set<juce::String> seen;
        hiddenDuplicates = 0;

        // built-in PEQUOD plugins first, so they win over a VST3 copy of the same plugin
        std::vector<int> scanOrder;
        for (int i = 0; i < types.size(); ++i)
            if (types.getReference (i).pluginFormatName == PequodNative::formatName) scanOrder.push_back (i);
        for (int i = 0; i < types.size(); ++i)
            if (types.getReference (i).pluginFormatName != PequodNative::formatName) scanOrder.push_back (i);

        for (int i : scanOrder)
        {
            auto& t = types.getReference (i);

            // The same plugin installed twice (e.g. in its collection and in PEQUOD Pedals): show it once.
            if (! seen.insert (PluginTags::duplicateKey (t)).second)
            {
                ++hiddenDuplicates;
                continue;
            }

            const auto function = PluginTags::functionOf (t);
            const auto collection = PluginTags::collectionOf (t);
            if (filter.isNotEmpty() && ! (t.name.containsIgnoreCase (filter) || t.manufacturerName.containsIgnoreCase (filter)
                                           || t.category.containsIgnoreCase (filter) || function.containsIgnoreCase (filter)
                                           || collection.containsIgnoreCase (filter)))
                continue;

            int order = 0;
            if (byFunction)                         order = PluginTags::functionOrder (t);
            else if (collection == "PEQUOD")        order = 0;   // the standalone PEQUOD tools first
            else if (collection == "PEQUOD Pedals") order = 1;
            else if (PluginTags::isPequod (t))      order = 2;
            else                                    order = 3;   // other makers last

            entries.push_back ({ i, byFunction ? function : collection, function, collection, order });
        }

        std::sort (entries.begin(), entries.end(), [&] (const Entry& a, const Entry& b)
        {
            if (a.order != b.order) return a.order < b.order;
            if (auto c = a.group.compareIgnoreCase (b.group); c != 0) return c < 0;
            auto& ta = types.getReference (a.index);
            auto& tb = types.getReference (b.index);
            if (auto c = ta.name.compareIgnoreCase (tb.name); c != 0) return c < 0;
            return a.collection.compareIgnoreCase (b.collection) < 0;
        });

        juce::String lastGroup = "\x01";
        for (auto& e : entries)
        {
            auto& t = types.getReference (e.index);
            const auto colour = PluginColours::forPlugin (t);
            if (e.group != lastGroup)
            {
                Row h { true, e.group.toUpperCase(), {}, -1, {}, {} };
                h.colour = byFunction && e.group != "Other" ? colour : juce::Colour();
                rows.push_back (h);
                lastGroup = e.group;
            }

            // The other tag goes on the right: the collection when grouped by function, and the reverse.
            juce::String tag;
            if (byFunction) tag = e.collection.upToFirstOccurrenceOf (PluginTags::sep(), false, false);
            else            tag = t.isInstrument ? juce::String ("Instrument") : e.function.upToFirstOccurrenceOf (",", false, false)
                                                                                    .upToFirstOccurrenceOf (" &", false, false);
            if (byFunction && t.isInstrument) tag = "Instrument" + PluginTags::sep() + tag;

            Row row { false, t.name, tag, e.index, {}, {} };
            row.colour = colour;
            rows.push_back (row);
        }

        if (rows.empty())
            rows.push_back ({ true, types.isEmpty() ? "No plugins - click PLUGINS > Scan" : "No matches", {}, -1, {}, {} });
    }

    list.updateContent();
    list.repaint();
}

void BrowserPanel::paintListBoxItem (int rowIndex, juce::Graphics& g, int w, int h, bool selected)
{
    if (! juce::isPositiveAndBelow (rowIndex, (int) rows.size())) return;
    auto& row = rows[(size_t) rowIndex];
    auto r = juce::Rectangle<int> (0, 0, w, h);

    if (row.isHeader)
    {
        if (row.bank.isNotEmpty())
        {
            const bool current = row.bank == library.getCurrentBank();
            auto band = r.reduced (2, 3);
            g.setColour (current ? Pequod::Colours::mustard : Pequod::Colours::panelHi);
            g.fillRoundedRectangle (band.toFloat(), 2.0f);
            g.setColour (current ? Pequod::Colours::bg : Pequod::Colours::mustard);
            g.setFont (Pequod::heading (12.0f));
            g.drawText ("BANK  " + row.text, band.reduced (8, 0), juce::Justification::centredLeft, true);
            g.setFont (Pequod::heading (10.0f, false));
            g.drawText (row.subText, band.reduced (8, 0), juce::Justification::centredRight);
            return;
        }

        if (row.text.startsWith ("   "))
        {
            g.setColour (Pequod::Colours::textDim);
            g.setFont (Pequod::body (12.0f));
            g.drawText (row.text.trim(), r.reduced (16, 0), juce::Justification::centredLeft);
            return;
        }

        const bool tinted = ! row.colour.isTransparent();
        g.setColour (tinted ? PluginColours::forLed (row.colour).withAlpha (0.6f) : Pequod::Colours::mustardLo.withAlpha (0.35f));
        g.fillRect (r.removeFromBottom (tinted ? 2 : 1));
        g.setColour (tinted ? PluginColours::forLed (row.colour) : Pequod::Colours::mustard);
        g.setFont (Pequod::heading (12.0f));
        g.drawText (row.text, r.reduced (10, 0), juce::Justification::bottomLeft);
        return;
    }

    const bool isCurrent = row.file != juce::File() && row.file == currentPreset;

    if (selected)
    {
        g.setColour (Pequod::Colours::panelHi);
        g.fillRect (r);
        g.setColour (Pequod::Colours::mustard);
        g.fillRect (r.withWidth (3));
    }

    if (row.pluginIndex >= 0 && ! row.colour.isTransparent())
    {
        // a small LED in the function's colour
        g.setColour (PluginColours::forLed (row.colour));
        g.fillEllipse (juce::Rectangle<float> (6.0f, 6.0f).withCentre ({ 10.0f, (float) h * 0.5f }));
    }

    g.setColour (isCurrent ? Pequod::Colours::mustard : Pequod::Colours::text);
    g.setFont (Pequod::body (14.0f, isCurrent));
    auto textArea = r.reduced (16, 0).withTrimmedLeft (row.pluginIndex >= 0 ? 4 : 0);
    if (row.subText.isNotEmpty())
    {
        auto sub = textArea.removeFromRight (juce::jmin (row.pluginIndex >= 0 ? 110 : 100, w * 2 / 5));
        g.setColour (Pequod::Colours::textDim);
        g.setFont (Pequod::heading (10.0f, false));
        g.drawText (row.subText.upToFirstOccurrenceOf ("|", false, false).toUpperCase(), sub, juce::Justification::centredRight);
        g.setColour (Pequod::Colours::text);
        g.setFont (Pequod::body (14.0f));
    }
    g.drawText (row.text, textArea, juce::Justification::centredLeft, true);
}

void BrowserPanel::listBoxItemDoubleClicked (int rowIndex, const juce::MouseEvent&)
{
    activateRow (rowIndex);
}

void BrowserPanel::activateRow (int rowIndex)
{
    if (! juce::isPositiveAndBelow (rowIndex, (int) rows.size())) return;
    auto& row = rows[(size_t) rowIndex];

    if (row.pluginIndex >= 0)
    {
        auto types = engine.knownPlugins.getTypes();
        if (juce::isPositiveAndBelow (row.pluginIndex, types.size()) && onAddPlugin)
            onAddPlugin (types.getReference (row.pluginIndex));
    }
    else if (row.file != juce::File() && onLoadPreset)
    {
        onLoadPreset (row.file);
    }
}

void BrowserPanel::listBoxItemClicked (int rowIndex, const juce::MouseEvent& e)
{
    if (! juce::isPositiveAndBelow (rowIndex, (int) rows.size())) return;
    auto row = rows[(size_t) rowIndex];

    if (row.bank.isNotEmpty())
    {
        if (e.mods.isPopupMenu()) showBankMenu (row.bank);
        else library.setCurrentBank (row.bank);
        return;
    }

    if (! e.mods.isPopupMenu()) return;

    if (row.pluginIndex >= 0)
    {
        juce::PopupMenu m;
        m.addItem ("Add to rack", [this, rowIndex] { activateRow (rowIndex); });
        m.showMenuAsync (juce::PopupMenu::Options());
    }
    else if (row.file != juce::File())
    {
        showPresetMenu (row.file);
    }
}

void BrowserPanel::showBankMenu (const juce::String& bank)
{
    juce::PopupMenu m;
    m.addItem ("Make current bank", [this, bank] { library.setCurrentBank (bank); });
    m.addItem ("Rename bank...", [this, bank] { if (onRenameBank) onRenameBank (bank); });
    m.addItem ("New bank...", [this] { if (onNewBank) onNewBank(); });
    m.addItem ("Show in folder", [this, bank] { library.getBankFolder (bank).revealToUser(); });
    m.addSeparator();
    m.addItem ("Delete bank...", library.getBanks().size() > 1, false, [this, bank] { if (onDeleteBank) onDeleteBank (bank); });
    m.showMenuAsync (juce::PopupMenu::Options());
}

void BrowserPanel::showPresetMenu (const juce::File& preset)
{
    juce::PopupMenu m;
    m.addItem ("Load", [this, preset] { if (onLoadPreset) onLoadPreset (preset); });

    juce::PopupMenu move;
    const auto from = PresetLibrary::bankOf (preset);
    for (auto& bank : library.getBanks())
        move.addItem (bank, bank != from, false, [this, preset, bank] { if (onMovePreset) onMovePreset (preset, bank); });
    m.addSubMenu ("Move to bank", move);

    m.addItem ("Show in folder", [preset] { preset.revealToUser(); });
    m.addSeparator();
    m.addItem ("Delete", [this, preset] { if (onDeletePreset) onDeletePreset (preset); });
    m.showMenuAsync (juce::PopupMenu::Options());
}

juce::var BrowserPanel::getDragSourceDescription (const juce::SparseSet<int>& selected)
{
    if (presetsTab || selected.isEmpty()) return {};
    auto rowIndex = selected[0];
    if (! juce::isPositiveAndBelow (rowIndex, (int) rows.size())) return {};
    auto& row = rows[(size_t) rowIndex];
    return row.pluginIndex >= 0 ? juce::var (dragIdForPlugin (row.pluginIndex)) : juce::var();
}

juce::String BrowserPanel::getTooltipForRow (int rowIndex)
{
    if (! juce::isPositiveAndBelow (rowIndex, (int) rows.size())) return {};
    auto& row = rows[(size_t) rowIndex];
    if (row.pluginIndex >= 0)
    {
        auto types = engine.knownPlugins.getTypes();
        if (juce::isPositiveAndBelow (row.pluginIndex, types.size()))
        {
            auto& t = types.getReference (row.pluginIndex);
            return t.name + "\n" + PluginTags::functionOf (t) + PluginTags::sep() + PluginTags::collectionOf (t)
                 + "\nDrag into the rack, or double-click to add at the end";
        }
        return "Drag into the rack, or double-click to add at the end";
    }
    if (row.isHeader && ! presetsTab && hiddenDuplicates > 0)
        return juce::String (hiddenDuplicates) + " duplicate cop" + (hiddenDuplicates == 1 ? "y" : "ies")
             + " of plugins installed twice are hidden";
    if (row.file != juce::File()) return "Double-click to load. Right-click to move it to another bank.";
    if (row.bank.isNotEmpty()) return "Click to make this the current bank. Right-click to rename or delete.";
    return {};
}

void BrowserPanel::paint (juce::Graphics& g)
{
    g.fillAll (Pequod::Colours::panel);
    g.setColour (Pequod::Colours::line);
    g.drawVerticalLine (getWidth() - 1, 0.0f, (float) getHeight());
}

void BrowserPanel::resized()
{
    auto r = getLocalBounds().reduced (8).withTrimmedRight (1);
    auto tabs = r.removeFromTop (30);
    pluginsTabButton.setBounds (tabs.removeFromLeft (tabs.getWidth() / 2).reduced (1, 0));
    presetsTabButton.setBounds (tabs.reduced (1, 0));
    r.removeFromTop (8);
    auto searchRow = r.removeFromTop (28);
    if (presetsTab)
    {
        newBankButton.setBounds (searchRow.removeFromRight (64));
        searchRow.removeFromRight (6);
    }
    search.setBounds (searchRow);
    r.removeFromTop (8);
    if (! presetsTab)
    {
        auto groupRow = r.removeFromTop (24);
        byFunctionButton.setBounds (groupRow.removeFromLeft (groupRow.getWidth() / 2).reduced (1, 0));
        byCollectionButton.setBounds (groupRow.reduced (1, 0));
        r.removeFromTop (6);
    }
    list.setBounds (r);
}
