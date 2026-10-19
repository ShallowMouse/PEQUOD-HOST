#include "BrowserPanel.h"

BrowserPanel::BrowserPanel (HostEngine& e, juce::File dir) : engine (e), presetDir (std::move (dir))
{
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
    showPresets (false);
}

BrowserPanel::~BrowserPanel()
{
    engine.knownPlugins.removeChangeListener (this);
}

void BrowserPanel::showPresets (bool p)
{
    presetsTab = p;
    presetsTabButton.setToggleState (p, juce::dontSendNotification);
    pluginsTabButton.setToggleState (! p, juce::dontSendNotification);
    refresh();
}

void BrowserPanel::refresh()
{
    rows.clear();
    const auto filter = search.getText().trim();

    if (presetsTab)
    {
        auto files = presetDir.findChildFiles (juce::File::findFiles, false, "*.pqrack");
        files.sort();
        for (auto& f : files)
            if (filter.isEmpty() || f.getFileNameWithoutExtension().containsIgnoreCase (filter))
                rows.push_back ({ false, f.getFileNameWithoutExtension(), {}, -1, f });

        if (rows.empty())
            rows.push_back ({ true, filter.isEmpty() ? "No presets yet - SAVE AS to make one" : "No matches", {}, -1, {} });
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
                rows.push_back ({ true, maker.toUpperCase(), {}, -1, {} });
                lastMaker = maker;
            }
            rows.push_back ({ false, t.name, t.isInstrument ? "INSTRUMENT" : t.category, i, {} });
        }

        if (rows.empty())
            rows.push_back ({ true, types.isEmpty() ? "No plugins - click PLUGINS > Scan" : "No matches", {}, -1, {} });
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
    if (! e.mods.isPopupMenu() || ! juce::isPositiveAndBelow (rowIndex, (int) rows.size())) return;
    auto row = rows[(size_t) rowIndex];

    juce::PopupMenu m;
    if (row.pluginIndex >= 0)
    {
        m.addItem ("Add to rack", [this, rowIndex] { activateRow (rowIndex); });
    }
    else if (row.file != juce::File())
    {
        m.addItem ("Load", [this, row] { if (onLoadPreset) onLoadPreset (row.file); });
        m.addItem ("Show in folder", [row] { row.file.revealToUser(); });
        m.addSeparator();
        m.addItem ("Delete", [this, row] { if (onDeletePreset) onDeletePreset (row.file); });
    }
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
    if (row.file != juce::File()) return "Double-click to load";
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
    search.setBounds (r.removeFromTop (28));
    r.removeFromTop (8);
    list.setBounds (r);
}
