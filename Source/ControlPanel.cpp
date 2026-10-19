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

    styleLabel (oscInLabel, "IN PORT");
    styleLabel (oscOutLabel, "SEND TO");
    styleLabel (inputModeLabel, "GUITAR INPUT");

    for (auto* ed : { &oscInPort, &oscOutHost, &oscOutPort })
    {
        ed->setFont (Pequod::heading (13.0f, false));
        ed->onReturnKey = [this] { applyOsc(); };
        ed->onFocusLost = [this] { applyOsc(); };
        addAndMakeVisible (ed);
    }
    oscInPort.setInputRestrictions (5, "0123456789");
    oscOutPort.setInputRestrictions (5, "0123456789");
    oscInPort.setText (juce::String (control.getOscInputPort()), false);
    oscOutHost.setText (control.getOscOutHost(), false);
    oscOutPort.setText (juce::String (control.getOscOutPort()), false);

    syncButton.setTooltip ("Send every slot name, on/off state and parameter value to the controller");
    syncButton.onClick = [this] { control.sendFullState(); };
    addAndMakeVisible (syncButton);

    inputMode.addItemList ({ "Stereo", "Left (input 1)", "Right (input 2)", "Left + Right" }, 1);
    inputMode.setSelectedItemIndex (engine.getInputStage()->inputMode.load(), juce::dontSendNotification);
    inputMode.onChange = [this] { engine.getInputStage()->inputMode = inputMode.getSelectedItemIndex(); };
    addAndMakeVisible (inputMode);

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
        m.addItem ("MUTE",         [this] { control.armLearnFor (Mapping::Target::mute); });
        m.showMenuAsync (juce::PopupMenu::Options().withTargetComponent (&hostLearnButton));
    };
    clearButton.setTooltip ("Remove all mappings for this rack");
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

void ControlPanel::styleLabel (juce::Label& l, const juce::String& text)
{
    l.setText (text, juce::dontSendNotification);
    l.setFont (Pequod::heading (11.0f, false));
    l.setColour (juce::Label::textColourId, Pequod::Colours::textDim);
    addAndMakeVisible (l);
}

void ControlPanel::applyOsc()
{
    auto in = oscInPort.getText().getIntValue();
    auto host = oscOutHost.getText().trim();
    auto out = oscOutPort.getText().getIntValue();

    if (in != control.getOscInputPort() || ! control.isOscInConnected())
        control.setOscInputPort (in);
    if (host != control.getOscOutHost() || out != control.getOscOutPort() || ! control.isOscOutConnected())
        control.setOscOutput (host, out);
}

void ControlPanel::changeListenerCallback (juce::ChangeBroadcaster*)
{
    if (! oscInPort.hasKeyboardFocus (false))  oscInPort.setText (juce::String (control.getOscInputPort()), false);
    if (! oscOutHost.hasKeyboardFocus (false)) oscOutHost.setText (control.getOscOutHost(), false);
    if (! oscOutPort.hasKeyboardFocus (false)) oscOutPort.setText (juce::String (control.getOscOutPort()), false);
    mappingList.updateContent();
    mappingList.repaint();
    mappingList.setVisible (! control.getMappings().empty());
    learnButton.setToggleState (control.getLearnState() != ControlSurface::LearnState::off, juce::dontSendNotification);
    repaint();
}

void ControlPanel::timerCallback()
{
    // Activity LEDs
    repaint (oscInLed.expanded (4));
    repaint (midiLed.expanded (4));

    if (inputMode.getSelectedItemIndex() != engine.getInputStage()->inputMode.load())
        inputMode.setSelectedItemIndex (engine.getInputStage()->inputMode.load(), juce::dontSendNotification);
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
    g.setColour (m.isRackMapping() ? Pequod::Colours::mustardLo : Pequod::Colours::line);
    g.fillRect (r.removeFromLeft (3));
    r.reduce (6, 2);

    auto top = r.removeFromTop (r.getHeight() / 2);
    g.setColour (Pequod::Colours::mustard);
    g.setFont (Pequod::heading (12.0f));
    g.drawText (control.describeSource (m), top, juce::Justification::centredLeft, true);

    if (m.isSwitchTarget() && m.source != Mapping::Source::osc)
    {
        g.setColour (Pequod::Colours::textDim);
        g.setFont (Pequod::heading (10.0f, false));
        g.drawText (m.toggle ? "TOGGLE" : "DIRECT", top, juce::Justification::centredRight);
    }

    g.setColour (Pequod::Colours::text);
    g.setFont (Pequod::body (12.0f));
    g.drawText ("-> " + control.describeTarget (m), r, juce::Justification::centredLeft, true);
}

void ControlPanel::listBoxItemClicked (int row, const juce::MouseEvent&)
{
    auto& maps = control.getMappings();
    if (! juce::isPositiveAndBelow (row, (int) maps.size())) return;

    juce::PopupMenu m;
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

    drawSectionTitle (g, oscSection, "OSC");
    drawSectionTitle (g, inputSection, "INPUT");
    drawSectionTitle (g, mapSection, "MAPPINGS");
    drawSectionTitle (g, chainSection, "EFFECTS MAP");

    drawLed (g, oscInLed, control.isOscInConnected(), recent (control.lastOscActivity));
    drawLed (g, oscOutLed, control.isOscOutConnected(), false);
    // MIDI activity LED beside the mappings title
    g.setFont (Pequod::heading (10.0f, false));
    g.setColour (Pequod::Colours::textDim);
    g.drawText ("MIDI", midiLed.withX (midiLed.getX() - 34).withWidth (30), juce::Justification::centredRight);
    drawLed (g, midiLed, false, recent (control.lastMidiActivity));

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

    // OSC
    oscSection = r.removeFromTop (24 + 26 + 6 + 26 + 6 + 26);
    {
        auto s = oscSection.withTrimmedTop (26);
        oscInLed = oscSection.withHeight (20).removeFromRight (14);
        auto row1 = s.removeFromTop (26);
        oscInLabel.setBounds (row1.removeFromLeft (64));
        oscInPort.setBounds (row1);
        s.removeFromTop (6);
        auto row2 = s.removeFromTop (26);
        oscOutLabel.setBounds (row2.removeFromLeft (64));
        oscOutLed = row2.removeFromRight (14);
        oscOutPort.setBounds (row2.removeFromRight (58));
        row2.removeFromRight (4);
        oscOutHost.setBounds (row2);
        s.removeFromTop (6);
        syncButton.setBounds (s.removeFromTop (26));
    }

    r.removeFromTop (14);
    inputSection = r.removeFromTop (24 + 26);
    {
        auto s = inputSection.withTrimmedTop (26);
        inputModeLabel.setBounds (s.removeFromLeft (90));
        inputMode.setBounds (s);
    }

    r.removeFromTop (14);
    const int mapHeight = juce::jmax (150, (r.getHeight() - 14) * 45 / 100);
    mapSection = r.removeFromTop (mapHeight);
    midiLed = mapSection.withHeight (20).removeFromRight (14);
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
