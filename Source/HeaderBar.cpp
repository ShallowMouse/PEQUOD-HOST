#include "HeaderBar.h"

//==============================================================================
void TunerDisplay::timerCallback()
{
    auto& r = tuner.getReading();
    if (r.valid != lastValid || r.midiNote != lastNote || std::abs (r.cents - lastCents) > 0.2f)
    {
        lastValid = r.valid;
        lastNote = r.midiNote;
        lastCents = r.cents;
        repaint();
    }
}

void TunerDisplay::mouseDown (const juce::MouseEvent& e)
{
    if (! e.mods.isPopupMenu()) return;

    juce::PopupMenu m;
    for (float a : { 432.0f, 436.0f, 438.0f, 440.0f, 442.0f, 444.0f })
        m.addItem ("A = " + juce::String ((int) a) + " Hz", true, std::abs (tuner.referenceA4 - a) < 0.1f,
                   [this, a] { tuner.referenceA4 = a; repaint(); });
    m.showMenuAsync (juce::PopupMenu::Options().withTargetComponent (this));
}

void TunerDisplay::paint (juce::Graphics& g)
{
    auto& r = tuner.getReading();
    auto bounds = getLocalBounds().toFloat();

    g.setColour (Pequod::Colours::well);
    g.fillRoundedRectangle (bounds, 3.0f);

    const bool inTune = r.valid && std::abs (r.cents) < 3.0f;
    g.setColour (inTune ? Pequod::Colours::mustard : Pequod::Colours::line);
    g.drawRoundedRectangle (bounds.reduced (0.5f), 3.0f, 1.0f);

    auto area = bounds.reduced (6.0f, 3.0f);

    // Note name
    auto noteArea = area.removeFromLeft (44.0f);
    g.setColour (r.valid ? (inTune ? Pequod::Colours::mustard : Pequod::Colours::text) : Pequod::Colours::textDim);
    g.setFont (Pequod::heading (24.0f));
    g.drawText (r.valid ? r.noteName : juce::String ("-"), noteArea.withTrimmedBottom (8.0f), juce::Justification::centred);
    g.setFont (Pequod::heading (10.0f, false));
    g.setColour (Pequod::Colours::textDim);
    g.drawText (r.valid ? juce::String (r.frequency, 1) + " Hz" : juce::String ("A" + juce::String ((int) tuner.referenceA4)),
                noteArea.removeFromBottom (11.0f), juce::Justification::centred);

    // Cents scale
    area.removeFromLeft (4.0f);
    auto scale = area.reduced (2.0f, 6.0f);
    const float cx = scale.getCentreX();
    const float halfW = scale.getWidth() * 0.5f;

    g.setColour (Pequod::Colours::textDim);
    for (int c = -50; c <= 50; c += 10)
    {
        const float x = cx + halfW * (float) c / 50.0f;
        const float len = c == 0 ? scale.getHeight() : (c % 50 == 0 ? scale.getHeight() * 0.6f : scale.getHeight() * 0.35f);
        g.drawLine (x, scale.getBottom() - len, x, scale.getBottom(), c == 0 ? 1.5f : 1.0f);
    }
    g.setFont (Pequod::heading (10.0f, false));
    g.drawText ("b", juce::Rectangle<float> (scale.getX(), scale.getY() - 4.0f, 12.0f, 12.0f), juce::Justification::centredLeft);
    g.drawText ("#", juce::Rectangle<float> (scale.getRight() - 12.0f, scale.getY() - 4.0f, 12.0f, 12.0f), juce::Justification::centredRight);

    if (r.valid)
    {
        const float x = cx + halfW * juce::jlimit (-50.0f, 50.0f, r.cents) / 50.0f;
        auto needle = juce::Rectangle<float> (4.0f, scale.getHeight() + 6.0f).withCentre ({ x, scale.getCentreY() });
        g.setColour (inTune ? Pequod::Colours::mustard : Pequod::Colours::text);
        g.fillRoundedRectangle (needle, 1.5f);
        if (inTune)
        {
            g.setColour (Pequod::Colours::mustard.withAlpha (0.25f));
            g.fillRoundedRectangle (needle.expanded (3.0f, 1.0f), 3.0f);
        }
    }
}

//==============================================================================
HeaderBar::HeaderBar (HostEngine& e, ControlSurface& c, Tuner& t)
    : engine (e), control (c), tunerDisplay (t)
{
    for (auto* b : std::initializer_list<juce::Button*> { &prevButton, &nextButton, &presetButton, &saveButton, &saveAsButton, &initButton,
                     &learnButton, &audioButton, &pluginsButton, &controllerButton })
        addAndMakeVisible (b);

    addAndMakeVisible (tunerDisplay);

    muteButton.setTooltip ("Mute the output (e.g. while tuning). Right-click to learn a MIDI / OSC control.");
    muteButton.onClick = [this] { control.setMuted (! control.isMuted()); };
    muteButton.onLearn = [this] { control.armLearnFor (Mapping::Target::mute); };
    muteButton.setColour (juce::TextButton::buttonOnColourId, Pequod::Colours::danger);
    muteButton.setColour (juce::TextButton::textColourOnId, Pequod::Colours::text);
    addAndMakeVisible (muteButton);

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
    controllerButton.onClick = [this] { if (onController) onController(); };
    controllerButton.setTooltip ("PEQUOD FC foot controller: what every switch does, and all its settings");

    setupKnob (inKnob, inLabel, "IN", engine.getInputStage(), Mapping::Target::inputGain);
    setupKnob (outKnob, outLabel, "OUT", engine.getOutputStage(), Mapping::Target::outputGain);

    addAndMakeVisible (inMeter);
    addAndMakeVisible (outMeter);

    cpuButton.setTooltip ("CPU load. Click to spread the effects chain over more CPU cores.");
    cpuButton.onClick = [this] { showCoresMenu(); };
    addAndMakeVisible (cpuButton);

    startTimerHz (30);
}

HeaderBar::~HeaderBar() { stopTimer(); }

void HeaderBar::ControllerButton::paintButton (juce::Graphics& g, bool over, bool down)
{
    TextButton::paintButton (g, over, down);
    auto r = getLocalBounds().toFloat();
    auto dot = juce::Rectangle<float> (8.0f, 8.0f).withCentre ({ r.getX() + 11.0f, r.getCentreY() });
    const auto c = state == 0 ? Pequod::Colours::textDim.withAlpha (0.4f)
                 : state == 2 ? Pequod::Colours::mustard : juce::Colour (0xff4caf50);
    if (state == 1)
    {
        g.setColour (c.withAlpha (0.3f));
        g.fillEllipse (dot.expanded (3.0f));
    }
    g.setColour (c);
    g.fillEllipse (dot);
}

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

    const int cs = controllerState ? controllerState() : 0;
    if (cs != controllerButton.state) { controllerButton.state = cs; controllerButton.repaint(); }

    if (! inKnob.isMouseButtonDown())  inKnob.setValue (in->gainDb.load(), juce::dontSendNotification);
    if (! outKnob.isMouseButtonDown()) outKnob.setValue (out->gainDb.load(), juce::dontSendNotification);

    learnButton.setToggleState (control.getLearnState() != ControlSurface::LearnState::off, juce::dontSendNotification);
    muteButton.setToggleState (control.isMuted(), juce::dontSendNotification);
    muteButton.setButtonText (control.isMuted() ? "MUTED" : "MUTE");

    static int tick = 0;
    if (++tick % 10 == 0)
    {
        const int cpu = juce::roundToInt (engine.deviceManager.getCpuUsage() * 100.0);
        const int stages = engine.getPipelineStages();
        cpuButton.top = "CPU " + juce::String (cpu) + "%";
        cpuButton.bottom = stages <= 1 ? juce::String ("1 CORE")
                                       : juce::String (stages) + " CORES +" + juce::String ((double) engine.getPipelineLatencySamples() * 1000.0
                                                                                               / juce::jmax (1.0, engine.getRack().getCurrentSampleRate()), 1) + "MS";
        cpuButton.hot = cpu >= 85;
        cpuButton.repaint();
    }
}

void HeaderBar::CpuButton::paintButton (juce::Graphics& g, bool over, bool)
{
    auto r = getLocalBounds().toFloat();
    if (over)
    {
        g.setColour (Pequod::Colours::panelHi);
        g.fillRoundedRectangle (r, 2.0f);
    }
    auto t = r.reduced (4.0f, 4.0f);
    g.setFont (Pequod::heading (12.0f));
    g.setColour (hot ? Pequod::Colours::danger : Pequod::Colours::text);
    g.drawText (top, t.removeFromTop (t.getHeight() * 0.55f), juce::Justification::bottomRight);
    g.setFont (Pequod::heading (9.5f, false));
    g.setColour (Pequod::Colours::mustard);
    g.drawText (bottom, t, juce::Justification::topRight);
}

void HeaderBar::showCoresMenu()
{
    const double sr = juce::jmax (1.0, engine.getRack().getCurrentSampleRate());
    const int block = engine.getRack().getCurrentBlockSize();

    juce::PopupMenu m;
    m.addSectionHeader ("SPREAD THE CHAIN OVER");
    for (int n = 1; n <= engine.getMaxPipelineStages(); ++n)
    {
        const double extraMs = (double) ((n - 1) * block) * 1000.0 / sr;
        auto text = n == 1 ? juce::String ("1 core  (no extra latency)")
                           : juce::String (n) + " cores  (+" + juce::String (extraMs, 1) + " ms latency)";
        m.addItem (text, true, engine.getPipelineStages() == n, [this, n]
        {
            if (onPipelineStagesChanged) onPipelineStagesChanged (n);
        });
    }
    m.addSeparator();
    m.addItem ("Split lanes always run on separate cores (no extra latency)", false, false, nullptr);
    m.addItem ("Busiest core now: " + juce::String (juce::roundToInt (engine.getBusiestCoreLoad() * 100.0f)) + "% of the buffer time",
               false, false, nullptr);
    m.showMenuAsync (juce::PopupMenu::Options().withTargetComponent (&cpuButton));
}

void HeaderBar::paint (juce::Graphics& g)
{
    auto r = getLocalBounds().toFloat();
    g.setGradientFill (juce::ColourGradient (juce::Colour (0xff262626), 0, 0, juce::Colour (0xff171717), 0, r.getHeight(), false));
    g.fillAll();
    g.setColour (Pequod::Colours::mustard);
    g.fillRect (r.removeFromBottom (2.0f));

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
    auto row = [] (juce::Rectangle<int> a, int h = 30) { return a.withSizeKeepingCentre (a.getWidth(), h); };

    // Right-hand block, laid out from the right edge: OUT, IN, CPU, PLUGINS, AUDIO, MUTE, tuner, LEARN
    auto knobBlock = [&] (LearnableSlider& k, juce::Label& l, PeakMeter& m)
    {
        auto b = r.removeFromRight (66);
        m.setBounds (b.removeFromRight (10).reduced (0, 10));
        b.removeFromRight (4);
        l.setBounds (b.removeFromBottom (14));
        k.setBounds (b.reduced (2, 2));
    };
    knobBlock (outKnob, outLabel, outMeter);
    r.removeFromRight (6);
    knobBlock (inKnob, inLabel, inMeter);
    r.removeFromRight (8);
    cpuButton.setBounds (row (r.removeFromRight (104), 40));
    r.removeFromRight (8);
    pluginsButton.setBounds (row (r.removeFromRight (78)));
    r.removeFromRight (4);
    controllerButton.setBounds (row (r.removeFromRight (126)));
    r.removeFromRight (4);
    audioButton.setBounds (row (r.removeFromRight (62)));
    r.removeFromRight (10);
    muteButton.setBounds (row (r.removeFromRight (66)));
    r.removeFromRight (4);
    tunerDisplay.setBounds (row (r.removeFromRight (176), 44));
    r.removeFromRight (10);
    learnButton.setBounds (row (r.removeFromRight (66)));
    r.removeFromRight (12);

    // Left-hand block: wordmark, preset navigation (preset name takes the remaining space)
    wordmarkArea = r.removeFromLeft (132);
    r.removeFromLeft (8);
    prevButton.setBounds (row (r.removeFromLeft (30)));
    r.removeFromLeft (4);

    auto saveAs = r.removeFromRight (74);
    r.removeFromRight (4);
    auto save = r.removeFromRight (52);
    r.removeFromRight (4);
    auto init = r.removeFromRight (48);
    r.removeFromRight (8);
    auto next = r.removeFromRight (30);
    r.removeFromRight (4);

    presetButton.setBounds (row (r.removeFromLeft (juce::jmin (r.getWidth(), 320))));
    nextButton.setBounds (row (next.withX (presetButton.getRight() + 4)));
    initButton.setBounds (row (init.withX (nextButton.getRight() + 8)));
    saveButton.setBounds (row (save.withX (initButton.getRight() + 4)));
    saveAsButton.setBounds (row (saveAs.withX (saveButton.getRight() + 4)));
}
