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
//==============================================================================
void TempoDisplay::paint (juce::Graphics& g)
{
    auto r = getLocalBounds().toFloat();
    g.setColour (Pequod::Colours::well);
    g.fillRoundedRectangle (r, 4.0f);
    g.setColour (transport.clickOn.load() ? Pequod::Colours::mustardLo : Pequod::Colours::line);
    g.drawRoundedRectangle (r.reduced (0.5f), 4.0f, 1.0f);

    auto inner = r.reduced (8.0f, 4.0f);
    auto top = inner.removeFromTop (12.0f);
    g.setFont (Pequod::heading (10.0f, false));
    g.setColour (Pequod::Colours::textDim);
    g.drawText (transport.clickOn.load() ? "TEMPO  -  CLICK ON" : "TEMPO", top, juce::Justification::centredLeft);

    // beat dots
    const int beats = juce::jlimit (1, 8, transport.beatsPerBar.load());
    const int now = transport.currentBeatInBar();
    const bool flash = transport.isOnBeatFlash();
    auto dots = inner.removeFromBottom (8.0f);
    const float d = 6.0f, gap = 4.0f;
    float x = dots.getRight() - (float) beats * (d + gap) + gap;
    for (int i = 0; i < beats; ++i, x += d + gap)
    {
        const bool lit = i == now && flash;
        g.setColour (lit ? (i == 0 ? Pequod::Colours::mustard : Pequod::Colours::text) : Pequod::Colours::line);
        g.fillEllipse (x, dots.getCentreY() - d * 0.5f, d, d);
    }

    g.setColour (Pequod::Colours::mustard);
    g.setFont (Pequod::heading (19.0f));
    auto bpmText = juce::String (transport.bpm.load(), 1);
    g.drawText (bpmText, inner.withTrimmedRight (2.0f), juce::Justification::centredLeft);
    g.setColour (Pequod::Colours::textDim);
    g.setFont (Pequod::heading (10.0f, false));
    g.drawText ("BPM", inner, juce::Justification::centredRight);
}

void TempoDisplay::timerCallback()
{
    const int beat = transport.isOnBeatFlash() ? transport.currentBeatInBar() : -1;
    if (beat != lastBeat || transport.bpm.load() != lastBpm || transport.clickOn.load() != lastClick)
    {
        lastBeat = beat;
        lastBpm = transport.bpm.load();
        lastClick = transport.clickOn.load();
        repaint();
    }
}

void TempoDisplay::mouseDown (const juce::MouseEvent& e)
{
    dragStartBpm = transport.bpm.load();
    dragged = false;
    if (e.mods.isPopupMenu()) { showMenu(); dragged = true; }
}

void TempoDisplay::mouseDrag (const juce::MouseEvent& e)
{
    if (std::abs (e.getDistanceFromDragStartY()) < 3 && ! dragged) return;
    dragged = true;
    const double perPixel = e.mods.isShiftDown() ? 0.1 : 0.5;
    transport.setBpm (dragStartBpm - perPixel * (double) e.getDistanceFromDragStartY());
    repaint();
}

void TempoDisplay::mouseUp (const juce::MouseEvent& e)
{
    if (dragged) { if (onChanged) onChanged(); return; }
    if (e.getNumberOfClicks() == 1)
    {
        juce::Component::SafePointer<TempoDisplay> safe (this);
        juce::Timer::callAfterDelay (260, [safe] { if (safe != nullptr && ! safe->dragged) safe->showMenu(); });
    }
}

void TempoDisplay::mouseDoubleClick (const juce::MouseEvent&)
{
    dragged = true;     // cancels the single-click menu
    typeTempo();
}

void TempoDisplay::showMenu()
{
    juce::PopupMenu m, volume, bars;
    for (int db : { -30, -24, -18, -12, -6, 0 })
        volume.addItem (juce::String (db) + " dB", true, std::abs (transport.clickGainDb.load() - (float) db) < 0.5f,
                        [this, db] { transport.clickGainDb = (float) db; if (onChanged) onChanged(); });
    for (int b = 2; b <= 7; ++b)
        bars.addItem (juce::String (b) + " beats", true, transport.beatsPerBar.load() == b,
                      [this, b] { transport.beatsPerBar = b; repaint(); if (onChanged) onChanged(); });
    m.addItem ("Metronome click", true, transport.clickOn.load(),
               [this] { transport.clickOn = ! transport.clickOn.load(); repaint(); if (onChanged) onChanged(); });
    m.addSubMenu ("Click volume", volume);
    m.addSubMenu ("Beats per bar", bars);
    m.addSeparator();
    m.addItem ("Type a tempo...", [this] { typeTempo(); });
    m.showMenuAsync (juce::PopupMenu::Options().withTargetComponent (this));
}

void TempoDisplay::typeTempo()
{
    auto* w = new juce::AlertWindow ("Tempo", "Beats per minute (" + juce::String ((int) Transport::minBpm) + " - "
                                     + juce::String ((int) Transport::maxBpm) + ")", juce::MessageBoxIconType::NoIcon);
    w->addTextEditor ("bpm", juce::String (transport.bpm.load(), 1));
    w->addButton ("OK", 1, juce::KeyPress (juce::KeyPress::returnKey));
    w->addButton ("Cancel", 0, juce::KeyPress (juce::KeyPress::escapeKey));
    juce::Component::SafePointer<TempoDisplay> safe (this);
    w->enterModalState (true, juce::ModalCallbackFunction::create ([safe, w] (int r)
    {
        if (r == 1 && safe != nullptr)
        {
            const double v = w->getTextEditorContents ("bpm").getDoubleValue();
            if (v > 0.0) { safe->transport.setBpm (v); safe->repaint(); if (safe->onChanged) safe->onChanged(); }
        }
    }), true);
}

//==============================================================================
HeaderBar::HeaderBar (HostEngine& e, ControlSurface& c, Tuner& t)
    : engine (e), control (c), tunerDisplay (t), tempoDisplay (e.transport)
{
    for (auto* b : std::initializer_list<juce::Button*> { &prevButton, &nextButton, &presetButton, &saveButton, &saveAsButton, &initButton,
                     &learnButton, &audioButton, &pluginsButton, &controllerButton })
        addAndMakeVisible (b);

    addAndMakeVisible (tunerDisplay);
    addAndMakeVisible (tempoDisplay);
    tempoDisplay.onChanged = [this] { if (onTempoSettingsChanged) onTempoSettingsChanged(); };
    tapButton.setTooltip ("Tap tempo: tap a few times in time. Right-click to learn a MIDI / OSC control (footswitches: CONTROLLER window).");
    tapButton.onClick = [this] { engine.transport.tap(); };
    tapButton.onLearn = [this] { control.armLearnFor (Mapping::Target::tapTempo); };
    addAndMakeVisible (tapButton);

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
    updateButton.setColour (juce::TextButton::buttonColourId, Pequod::Colours::mustard);
    updateButton.setColour (juce::TextButton::textColourOffId, Pequod::Colours::rail);
    updateButton.onClick = [this] { if (onUpdate) onUpdate(); };
    addChildComponent (updateButton);
    controllerButton.setTooltip ("PEQUOD FC foot controller: what every switch does, and all its settings");

    setupKnob (inKnob, inLabel, "IN", engine.getInputStage(), Mapping::Target::inputGain);
    setupKnob (outKnob, outLabel, "OUT", engine.getOutputStage(), Mapping::Target::outputGain);

    addAndMakeVisible (inMeter);
    addAndMakeVisible (outMeter);
    outMeter.showsClip = true;
    outMeter.onClearClip = [this] { engine.getOutputStage()->clipped = false; };
    outMeter.setTooltip ("Output level. The light on top turns red if the output hit 0 dB (click to clear); "
                         "mustard while the safety limiter is working.");
    outKnob.setTooltip ("OUT level, saved with each preset (right-click: learn, match levels, limiter)");
    outKnob.addToMenu = [this] (juce::PopupMenu& m) { addOutputMenu (m); };

    cpuButton.setTooltip ("CPU load. Click to spread the effects chain over more CPU cores.");
    cpuButton.onClick = [this] { showCoresMenu(); };
    addAndMakeVisible (cpuButton);

    startTimerHz (30);
}

HeaderBar::~HeaderBar() { stopTimer(); }

void HeaderBar::setUpdate (const juce::String& version, bool downloading, float progress)
{
    const bool show = version.isNotEmpty();
    updateButton.setButtonText (downloading ? "DOWNLOADING " + juce::String (juce::roundToInt (progress * 100.0f)) + "%"
                                            : "UPDATE " + version);
    updateButton.setTooltip ("PEQUOD Host " + version + " is available. Click to download and install it - PEQUOD restarts by itself.");
    updateButton.setEnabled (! downloading);
    if (show != updateButton.isVisible()) { updateButton.setVisible (show); resized(); }
}

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

//==============================================================================
void HeaderBar::addOutputMenu (juce::PopupMenu& m)
{
    auto* out = engine.getOutputStage();
    const bool hasRef = ! std::isnan (engine.loudnessReference);

    m.addSeparator();
    m.addSectionHeader ("MATCH LEVELS BETWEEN PRESETS");
    m.addItem ("Use this preset's level as the target (play for " + juce::String ((int) measureSeconds) + " s)",
               measuring == 0, false, [this] { startLevelMatch (1); });
    m.addItem (hasRef ? "Match this preset to the target (play for " + juce::String ((int) measureSeconds) + " s)"
                      : juce::String ("Match this preset to the target (set a target first)"),
               hasRef && measuring == 0, false, [this] { startLevelMatch (2); });
    if (measuring != 0)
        m.addItem ("Stop measuring", [this] { measuring = 0; outLabel.setText ("OUT", juce::dontSendNotification); });
    m.addSeparator();
    m.addItem ("Safety limiter on the output", true, out->limiterOn.load(), [out]
    {
        out->limiterOn = ! out->limiterOn.load();
    });
}

void HeaderBar::startLevelMatch (int mode)
{
    measuring = mode;
    measureStart = juce::Time::getMillisecondCounterHiRes();
    engine.getOutputStage()->startLoudness();
    if (onStatus) onStatus ("Measuring the level: play the way you normally would with this sound...");
}

void HeaderBar::updateLevelMatch()
{
    if (measuring == 0) return;
    auto* out = engine.getOutputStage();
    const double elapsed = (juce::Time::getMillisecondCounterHiRes() - measureStart) / 1000.0;
    const int left = juce::jmax (0, (int) std::ceil (measureSeconds - elapsed));
    outLabel.setText ("PLAY " + juce::String (left), juce::dontSendNotification);
    if (elapsed < measureSeconds) return;

    const int mode = measuring;
    measuring = 0;
    outLabel.setText ("OUT", juce::dontSendNotification);

    if (out->secondsMeasured() < 1.5)
    {
        if (onStatus) onStatus ("Didn't hear enough playing to measure the level. Try again and keep playing the whole time.");
        return;
    }

    const double heard = out->loudnessDb();            // before the OUT level
    if (mode == 1)
    {
        engine.loudnessReference = heard + out->gainDb.load();
        if (onLevelTargetChanged) onLevelTargetChanged();
        if (onStatus) onStatus ("Level target set from this preset. On other presets, right-click OUT > Match.");
    }
    else
    {
        const float before = out->gainDb.load();
        const float after = (float) juce::jlimit (-36.0, 12.0, std::round ((engine.loudnessReference - heard) * 10.0) / 10.0);
        out->gainDb = after;
        engine.markModified();
        if (onStatus) onStatus ("Level matched: OUT " + juce::String (before, 1) + " dB -> " + juce::String (after, 1)
                                + " dB. Save the preset to keep it.");
    }
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
    outMeter.setClip (out->clipped.load(), out->reductionDb.load());
    updateLevelMatch();

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

    if (! showWordmark) return;
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
    auto full = getLocalBounds().reduced (14, 0).withTrimmedBottom (2);
    auto row = [] (juce::Rectangle<int> a, int h = 30) { return a.withSizeKeepingCentre (a.getWidth(), h); };

    // Widths per size: normal, compact; the wordmark goes last when even compact is too wide.
    struct Sizes { int cpu, controller, gap, tuner, tap, tempo, wordmark, update; };
    const Sizes normal  { 104, 126, 10, 176, 50, 124, 132, 150 };
    const Sizes compact {  78, 112,  6, 140, 44, 108, 132, 112 };

    auto needed = [&] (const Sizes& z, bool wordmark)
    {
        int w = 66 + 6 + 66 + 8 + z.cpu + 8 + 78 + 4 + z.controller + 4 + 62 + z.gap + 66 + 4 + z.tuner + z.gap
              + 66 + z.gap + z.tap + 4 + z.tempo + 12;
        w += (wordmark ? z.wordmark + 8 : 0) + (updateButton.isVisible() ? z.update + 8 : 0);
        w += 30 + 4 + 4 + 30 + 8 + 48 + 4 + 52 + 4 + 74;   // < > INIT SAVE SAVE AS
        return w;
    };
    constexpr int presetWanted = 200;
    const bool useCompact = needed (normal, true) + presetWanted > full.getWidth();
    const Sizes& z = useCompact ? compact : normal;
    showWordmark = needed (z, true) + 150 <= full.getWidth();

    auto r = full;
    // Right-hand block, laid out from the right edge: OUT, IN, CPU, PLUGINS, CONTROLLER, AUDIO, MUTE, tuner, LEARN, TAP, tempo
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
    cpuButton.setBounds (row (r.removeFromRight (z.cpu), 40));
    r.removeFromRight (8);
    pluginsButton.setBounds (row (r.removeFromRight (78)));
    r.removeFromRight (4);
    controllerButton.setBounds (row (r.removeFromRight (z.controller)));
    r.removeFromRight (4);
    audioButton.setBounds (row (r.removeFromRight (62)));
    r.removeFromRight (z.gap);
    muteButton.setBounds (row (r.removeFromRight (66)));
    r.removeFromRight (4);
    tunerDisplay.setBounds (row (r.removeFromRight (z.tuner), 44));
    r.removeFromRight (z.gap);
    learnButton.setBounds (row (r.removeFromRight (66)));
    r.removeFromRight (z.gap);
    tapButton.setBounds (row (r.removeFromRight (z.tap)));
    r.removeFromRight (4);
    tempoDisplay.setBounds (row (r.removeFromRight (z.tempo), 44));
    r.removeFromRight (12);

    // Left-hand block: wordmark, preset navigation (preset name takes the remaining space)
    wordmarkArea = showWordmark ? r.removeFromLeft (z.wordmark) : juce::Rectangle<int>();
    if (showWordmark) r.removeFromLeft (8);
    if (updateButton.isVisible())
    {
        updateButton.setBounds (row (r.removeFromLeft (z.update)));
        r.removeFromLeft (8);
    }
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

    presetButton.setBounds (row (r.removeFromLeft (juce::jlimit (0, 320, r.getWidth()))));
    nextButton.setBounds (row (next.withX (presetButton.getRight() + 4)));
    initButton.setBounds (row (init.withX (nextButton.getRight() + 8)));
    saveButton.setBounds (row (save.withX (initButton.getRight() + 4)));
    saveAsButton.setBounds (row (saveAs.withX (saveButton.getRight() + 4)));
}
