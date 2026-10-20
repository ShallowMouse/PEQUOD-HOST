#include "ControlPanel.h"

namespace
{
    void drawSectionTitle (juce::Graphics& g, juce::Rectangle<int> section, const juce::String& title)
    {
        auto t = section.withHeight (20);
        g.setColour (Pequod::Colours::mustard);
        g.setFont (Pequod::heading (13.0f));
        g.drawText (title, t, juce::Justification::centredLeft);
        g.setColour (Pequod::Colours::mustardLo.withAlpha (0.5f));
        g.fillRect (t.getX(), t.getBottom(), t.getWidth(), 1);
    }

    void drawLed (juce::Graphics& g, juce::Rectangle<int> r, bool on, bool flash)
    {
        auto c = r.toFloat().withSizeKeepingCentre (9, 9);
        g.setColour (flash ? Pequod::Colours::text : on ? Pequod::Colours::mustard : juce::Colour (0xff3a3a3a));
        g.fillEllipse (c);
    }

    bool recent (double t) { return juce::Time::getMillisecondCounterHiRes() - t < 120.0; }
}

ControlPanel::ControlPanel (HostEngine& e, ControlSurface& c) : engine (e), control (c), chainMap (e)
{
    chainMap.onSelect = [this] (int uid) { if (onShowInRack) onShowInRack (uid); };
    chainMap.isMuted = [this] { return control.isMuted(); };
    chainViewport.setViewedComponent (&chainMap, false);
    chainViewport.setScrollBarsShown (true, false);
    chainViewport.setScrollBarThickness (8);
    addAndMakeVisible (chainViewport);

    learnButton.setTooltip ("Move a control on a plugin, then a MIDI control (or send an OSC message)");
    learnButton.onClick = [this]
    {
        if (control.getLearnState() != ControlSurface::LearnState::off) control.cancelLearn();
        else control.armLearn();
    };
    hostLearnButton.setTooltip ("Learn a MIDI / OSC control for a host function");
    hostLearnButton.onClick = [this]
    {
        juce::PopupMenu m;
        m.addItem ("Preset NEXT",  [this] { control.armLearnFor (Mapping::Target::presetNext); });
        m.addItem ("Preset PREV",  [this] { control.armLearnFor (Mapping::Target::presetPrev); });
        m.addItem ("Master IN",    [this] { control.armLearnFor (Mapping::Target::inputGain); });
        m.addItem ("Master OUT",   [this] { control.armLearnFor (Mapping::Target::outputGain); });
        m.addItem ("Bank NEXT",    [this] { control.armLearnFor (Mapping::Target::bankNext); });
        m.addItem ("Bank PREV",    [this] { control.armLearnFor (Mapping::Target::bankPrev); });
        m.addItem ("MUTE",         [this] { control.armLearnFor (Mapping::Target::mute); });
        m.showMenuAsync (juce::PopupMenu::Options().withTargetComponent (&hostLearnButton));
    };
    clearButton.setTooltip ("Remove this preset's own mappings (the ones for every preset stay)");
    clearButton.onClick = [this] { control.clearRackMappings(); };

    for (auto* b : { &learnButton, &hostLearnButton, &clearButton })
        addAndMakeVisible (b);

    mappingList.setRowHeight (38);
    addChildComponent (mappingList);
    mappingList.setVisible (! control.getMappings().empty());

    control.addChangeListener (this);
    startTimerHz (20);
}

ControlPanel::~ControlPanel()
{
    stopTimer();
    control.removeChangeListener (this);
    chainViewport.setViewedComponent (nullptr, false);
}

void ControlPanel::changeListenerCallback (juce::ChangeBroadcaster*)
{
    mappingList.updateContent();
    mappingList.repaint();
    mappingList.setVisible (! control.getMappings().empty());
    learnButton.setToggleState (control.getLearnState() != ControlSurface::LearnState::off, juce::dontSendNotification);
    repaint();
}

void ControlPanel::timerCallback()
{
    // Activity LEDs
    repaint (oscLed.expanded (40, 4));
    repaint (midiLed.expanded (40, 4));
}

//==============================================================================
int ControlPanel::getNumRows() { return (int) control.getMappings().size(); }

void ControlPanel::paintListBoxItem (int row, juce::Graphics& g, int w, int h, bool selected)
{
    auto& maps = control.getMappings();
    if (! juce::isPositiveAndBelow (row, (int) maps.size())) return;
    auto& m = maps[(size_t) row];

    auto r = juce::Rectangle<int> (0, 0, w, h).reduced (2, 2);
    g.setColour (selected ? Pequod::Colours::panelHi : Pequod::Colours::panel);
    g.fillRect (r);
    g.setColour (m.perPreset ? Pequod::Colours::mustardLo : Pequod::Colours::line);
    g.fillRect (r.removeFromLeft (3));
    r.reduce (6, 2);

    const bool overridden = control.isOverridden (m);
    auto top = r.removeFromTop (r.getHeight() / 2);
    auto tags = top.removeFromRight (m.isSwitchTarget() && m.source != Mapping::Source::osc ? 104 : 52);

    g.setFont (Pequod::heading (10.0f, false));
    g.setColour (m.perPreset ? Pequod::Colours::mustard : Pequod::Colours::textDim);
    g.drawText (m.perPreset ? "PRESET" : "ALL", tags.removeFromRight (48), juce::Justification::centredRight);
    if (m.isSwitchTarget() && m.source != Mapping::Source::osc)
    {
        g.setColour (Pequod::Colours::textDim);
        g.drawText (m.toggle ? "TOGGLE" : "DIRECT", tags, juce::Justification::centredRight);
    }

    g.setColour (overridden ? Pequod::Colours::textDim : Pequod::Colours::mustard);
    g.setFont (Pequod::heading (12.0f));
    g.drawText (control.describeSource (m), top, juce::Justification::centredLeft, true);

    g.setColour (overridden ? Pequod::Colours::textDim : Pequod::Colours::text);
    g.setFont (Pequod::body (12.0f));
    juce::String target = "-> " + control.describeTarget (m);
    if (overridden) target << "   (this preset uses its own)";
    else if (! m.perPreset && m.isRackMapping() && m.slotUid == 0) target << "   (not in this preset)";
    g.drawText (target, r, juce::Justification::centredLeft, true);
}

void ControlPanel::listBoxItemClicked (int row, const juce::MouseEvent&)
{
    auto& maps = control.getMappings();
    if (! juce::isPositiveAndBelow (row, (int) maps.size())) return;

    juce::PopupMenu m;
    const bool perPreset = maps[(size_t) row].perPreset;
    m.addItem ("Only in this preset", true, perPreset, [this, row] { control.setMappingScope (row, true); });
    m.addItem ("In every preset", ! maps[(size_t) row].isRackMapping() || maps[(size_t) row].slotUid != 0, ! perPreset,
               [this, row] { control.setMappingScope (row, false); });
    m.addSeparator();
    if (maps[(size_t) row].isSwitchTarget())
    {
        m.addItem ("Toggle on each press", true, maps[(size_t) row].toggle, [this, row] { if (! control.getMappings()[(size_t) row].toggle) control.toggleMappingMode (row); });
        m.addItem ("Direct (high = on, low = off)", true, ! maps[(size_t) row].toggle, [this, row] { if (control.getMappings()[(size_t) row].toggle) control.toggleMappingMode (row); });
        m.addSeparator();
    }
    m.addItem ("Delete mapping", [this, row] { control.removeMapping (row); });
    m.showMenuAsync (juce::PopupMenu::Options());
}

//==============================================================================
void ControlPanel::paint (juce::Graphics& g)
{
    g.fillAll (Pequod::Colours::panel);
    g.setColour (Pequod::Colours::line);
    g.drawVerticalLine (0, 0.0f, (float) getHeight());

    drawSectionTitle (g, mapSection, "MAPPINGS");
    drawSectionTitle (g, chainSection, "EFFECTS MAP");

    // MIDI activity LED beside the mappings title
    g.setFont (Pequod::heading (10.0f, false));
    g.setColour (Pequod::Colours::textDim);
    g.drawText ("MIDI", midiLed.withX (midiLed.getX() - 34).withWidth (30), juce::Justification::centredRight);
    drawLed (g, midiLed, false, recent (control.lastMidiActivity));
    g.setFont (Pequod::heading (10.0f, false));
    g.setColour (Pequod::Colours::textDim);
    g.drawText ("OSC", oscLed.withX (oscLed.getX() - 34).withWidth (30), juce::Justification::centredRight);
    drawLed (g, oscLed, control.isOscInConnected(), recent (control.lastOscActivity));

    if (control.getMappings().empty())
    {
        g.setColour (Pequod::Colours::textDim);
        g.setFont (Pequod::body (12.0f));
        g.setColour (Pequod::Colours::well);
        g.fillRect (mappingList.getBounds());
        g.setColour (Pequod::Colours::textDim);
        g.drawFittedText ("Click LEARN, move a knob on any plugin, then move a MIDI control or send OSC.\n\n"
                          "Right-click an on/off LED, MUTE, the IN/OUT knobs or a split fader to learn those.",
                          mappingList.getBounds().reduced (8), juce::Justification::topLeft, 8);
    }
}

void ControlPanel::resized()
{
    auto r = getLocalBounds().reduced (12, 10).withTrimmedLeft (1);

    r.removeFromTop (2);
    const int mapHeight = juce::jmax (180, (r.getHeight() - 14) * 50 / 100);
    mapSection = r.removeFromTop (mapHeight);
    {
        auto title = mapSection.withHeight (20);
        oscLed = title.removeFromRight (14);
        title.removeFromRight (40);
        midiLed = title.removeFromRight (14);
    }
    r.removeFromTop (14);
    chainSection = r;
    chainViewport.setBounds (chainSection.withTrimmedTop (26));
    chainMap.setSize (chainViewport.getWidth() - chainViewport.getScrollBarThickness(),
                      juce::jmax (chainMap.getDesiredHeight(), chainViewport.getHeight()));

    auto s = mapSection.withTrimmedTop (26);
    auto buttons = s.removeFromTop (26);
    auto bw = (buttons.getWidth() - 8) / 3;
    learnButton.setBounds (buttons.removeFromLeft (bw));
    buttons.removeFromLeft (4);
    hostLearnButton.setBounds (buttons.removeFromLeft (bw));
    buttons.removeFromLeft (4);
    clearButton.setBounds (buttons);
    s.removeFromTop (6);
    mappingList.setBounds (s);
}
