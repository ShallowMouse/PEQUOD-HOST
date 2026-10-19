#include "HeaderBar.h"

HeaderBar::HeaderBar (HostEngine& e, ControlSurface& c) : engine (e), control (c)
{
    for (auto* b : { &prevButton, &nextButton, &presetButton, &saveButton, &saveAsButton, &initButton,
                     &learnButton, &audioButton, &pluginsButton })
        addAndMakeVisible (b);

    presetButton.setColour (juce::TextButton::buttonColourId, Pequod::Colours::well);
    presetButton.setTooltip ("Click for the preset list");
    learnButton.setTooltip ("MIDI / OSC learn: click, move a control on any plugin, then move a MIDI control or send OSC");
    initButton.setTooltip ("Empty rack");

    prevButton.onClick    = [this] { if (onPrev) onPrev(); };
    nextButton.onClick    = [this] { if (onNext) onNext(); };
    presetButton.onClick  = [this] { if (onPresetMenu) onPresetMenu(); };
    saveButton.onClick    = [this] { if (onSave) onSave(); };
    saveAsButton.onClick  = [this] { if (onSaveAs) onSaveAs(); };
    initButton.onClick    = [this] { if (onInit) onInit(); };
    learnButton.onClick   = [this] { if (onToggleLearn) onToggleLearn(); };
    audioButton.onClick   = [this] { if (onAudioSettings) onAudioSettings(); };
    pluginsButton.onClick = [this] { if (onPluginManager) onPluginManager(); };

    setupKnob (inKnob, inLabel, "IN", engine.getInputStage(), Mapping::Target::inputGain);
    setupKnob (outKnob, outLabel, "OUT", engine.getOutputStage(), Mapping::Target::outputGain);

    addAndMakeVisible (inMeter);
    addAndMakeVisible (outMeter);

    cpuLabel.setFont (Pequod::heading (12.0f, false));
    cpuLabel.setColour (juce::Label::textColourId, Pequod::Colours::textDim);
    cpuLabel.setJustificationType (juce::Justification::centredRight);
    addAndMakeVisible (cpuLabel);

    startTimerHz (30);
}

HeaderBar::~HeaderBar() { stopTimer(); }

void HeaderBar::setupKnob (LearnableSlider& knob, juce::Label& label, const juce::String& name,
                           GainMeterProcessor* stage, Mapping::Target target)
{
    knob.setSliderStyle (juce::Slider::RotaryHorizontalVerticalDrag);
    knob.setTextBoxStyle (juce::Slider::NoTextBox, false, 0, 0);
    knob.setRange (-36.0, 12.0, 0.1);
    knob.setDoubleClickReturnValue (true, 0.0);
    knob.setTextValueSuffix (" dB");
    knob.setPopupDisplayEnabled (true, true, this);
    knob.setValue (stage->gainDb.load(), juce::dontSendNotification);
    knob.onValueChange = [&knob, stage] { stage->gainDb = (float) knob.getValue(); };
    knob.onLearn = [this, target] { control.armLearnFor (target); };
    knob.setTooltip (name + " level (right-click to learn)");
    addAndMakeVisible (knob);

    label.setText (name, juce::dontSendNotification);
    label.setFont (Pequod::heading (11.0f));
    label.setColour (juce::Label::textColourId, Pequod::Colours::textDim);
    label.setJustificationType (juce::Justification::centred);
    addAndMakeVisible (label);
}

void HeaderBar::setPresetName (const juce::String& name, bool modified)
{
    presetButton.setButtonText (name.toUpperCase() + (modified ? " *" : ""));
}

void HeaderBar::timerCallback()
{
    auto* in = engine.getInputStage();
    auto* out = engine.getOutputStage();
    inMeter.setLevels (in->peakL, in->peakR);
    outMeter.setLevels (out->peakL, out->peakR);

    if (! inKnob.isMouseButtonDown())  inKnob.setValue (in->gainDb.load(), juce::dontSendNotification);
    if (! outKnob.isMouseButtonDown()) outKnob.setValue (out->gainDb.load(), juce::dontSendNotification);

    learnButton.setToggleState (control.getLearnState() != ControlSurface::LearnState::off, juce::dontSendNotification);

    static int tick = 0;
    if (++tick % 10 == 0)
        cpuLabel.setText ("CPU " + juce::String (juce::roundToInt (engine.deviceManager.getCpuUsage() * 100.0)) + "%",
                          juce::dontSendNotification);
}

void HeaderBar::paint (juce::Graphics& g)
{
    auto r = getLocalBounds().toFloat();
    g.setGradientFill (juce::ColourGradient (juce::Colour (0xff262626), 0, 0, juce::Colour (0xff171717), 0, r.getHeight(), false));
    g.fillAll();
    g.setColour (Pequod::Colours::mustard);
    g.fillRect (r.removeFromBottom (2.0f));

    // Wordmark
    auto wm = wordmarkArea.toFloat();
    g.setColour (Pequod::Colours::mustard);
    g.setFont (Pequod::heading (28.0f));
    g.drawText ("PEQUOD", wm.removeFromTop (wm.getHeight() * 0.68f), juce::Justification::bottomLeft);
    g.setColour (Pequod::Colours::textDim);
    g.setFont (Pequod::heading (11.0f, false));
    g.drawText ("H  O  S  T", wm, juce::Justification::topLeft);
}

void HeaderBar::resized()
{
    auto r = getLocalBounds().reduced (14, 0).withTrimmedBottom (2);
    wordmarkArea = r.removeFromLeft (150);

    auto row = [&] (juce::Rectangle<int> a) { return a.withSizeKeepingCentre (a.getWidth(), 30); };

    r.removeFromLeft (10);
    prevButton.setBounds (row (r.removeFromLeft (30)));
    r.removeFromLeft (4);
    presetButton.setBounds (row (r.removeFromLeft (juce::jlimit (160, 300, getWidth() / 5))));
    r.removeFromLeft (4);
    nextButton.setBounds (row (r.removeFromLeft (30)));
    r.removeFromLeft (10);
    initButton.setBounds (row (r.removeFromLeft (52)));
    r.removeFromLeft (4);
    saveButton.setBounds (row (r.removeFromLeft (56)));
    r.removeFromLeft (4);
    saveAsButton.setBounds (row (r.removeFromLeft (76)));
    r.removeFromLeft (16);
    learnButton.setBounds (row (r.removeFromLeft (70)));

    // Right side: OUT, IN, CPU, settings
    auto knobBlock = [&] (LearnableSlider& k, juce::Label& l, PeakMeter& m)
    {
        auto b = r.removeFromRight (70);
        m.setBounds (b.removeFromRight (10).reduced (0, 10));
        b.removeFromRight (4);
        l.setBounds (b.removeFromBottom (14));
        k.setBounds (b.reduced (2, 2));
    };
    knobBlock (outKnob, outLabel, outMeter);
    r.removeFromRight (8);
    knobBlock (inKnob, inLabel, inMeter);
    r.removeFromRight (12);
    cpuLabel.setBounds (row (r.removeFromRight (70)));
    r.removeFromRight (8);
    pluginsButton.setBounds (row (r.removeFromRight (80)));
    r.removeFromRight (4);
    audioButton.setBounds (row (r.removeFromRight (66)));
}
