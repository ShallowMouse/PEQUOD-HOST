#include "BrowserPanel.h"

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
    resized();
    refresh();
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
        std::vector<int> order;
        for (int i = 0; i < types.size(); ++i)
        {
            auto& t = types.getReference (i);
            if (filter.isEmpty() || t.name.containsIgnoreCase (filter) || t.manufacturerName.containsIgnoreCase (filter)
                 || t.category.containsIgnoreCase (filter))
                order.push_back (i);
        }

        std::sort (order.begin(), order.end(), [&] (int a, int b)
        {
            auto& ta = types.getReference (a);
            auto& tb = types.getReference (b);
            auto c = ta.manufacturerName.compareIgnoreCase (tb.manufacturerName);
            return c != 0 ? c < 0 : ta.name.compareIgnoreCase (tb.name) < 0;
        });

        juce::String lastMaker = "\x01";
        for (auto i : order)
        {
            auto& t = types.getReference (i);
            auto maker = t.manufacturerName.isEmpty() ? juce::String ("Unknown") : t.manufacturerName;
            if (maker != lastMaker)
            {
                rows.push_back ({ true, maker.toUpperCase(), {}, -1, {}, {} });
                lastMaker = maker;
            }
            rows.push_back ({ false, t.name, t.isInstrument ? "INSTRUMENT" : t.category, i, {}, {} });
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

        g.setColour (Pequod::Colours::mustardLo.withAlpha (0.35f));
        g.fillRect (r.removeFromBottom (1));
        g.setColour (Pequod::Colours::mustard);
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

    g.setColour (isCurrent ? Pequod::Colours::mustard : Pequod::Colours::text);
    g.setFont (Pequod::body (14.0f, isCurrent));
    auto textArea = r.reduced (16, 0);
    if (row.subText.isNotEmpty())
    {
        auto sub = textArea.removeFromRight (juce::jmin (100, w / 3));
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
    if (row.pluginIndex >= 0) return "Drag into the rack, or double-click to add at the end";
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
    list.setBounds (r);
}
