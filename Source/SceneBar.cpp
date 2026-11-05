#include "SceneBar.h"

namespace
{
    void styleButton (juce::TextButton& b)
    {
        b.setColour (juce::TextButton::buttonColourId, Pequod::Colours::well);
        b.setColour (juce::TextButton::buttonOnColourId, Pequod::Colours::mustard);
        b.setColour (juce::TextButton::textColourOnId, Pequod::Colours::rail);
    }
}

SceneBar::SceneBar (HostEngine& e, ControlSurface& c) : engine (e), control (c)
{
    styleButton (addButton);
    addButton.setTooltip ("Add a scene: a version of this preset (effects on/off, knobs, split mixes, levels, modifiers on/off) "
                          "you can switch to instantly. It starts as a copy of how it sounds now.");
    addButton.onClick = [this]
    {
        if (engine.addScene() < 0)
            juce::AlertWindow::showMessageBoxAsync (juce::MessageBoxIconType::NoIcon, "Scenes",
                                                    "A preset can have up to " + juce::String (HostEngine::maxScenes) + " scenes.");
        else if (onStatus)
            onStatus ("New scene: change effects and knobs now - the changes belong to this scene. Click a scene to switch.");
    };
    addAndMakeVisible (addButton);

    auto setupKnob = [this] (LearnableSlider& k, juce::Label& l, const juce::String& name, GainMeterProcessor* stage, Mapping::Target target)
    {
        k.setSliderStyle (juce::Slider::RotaryHorizontalVerticalDrag);
        k.setTextBoxStyle (juce::Slider::NoTextBox, false, 0, 0);
        k.setRange (-36.0, 12.0, 0.1);
        k.setDoubleClickReturnValue (true, 0.0);
        k.setValue (stage->presetDb.load(), juce::dontSendNotification);
        k.onValueChange = [this, &k, stage] { stage->presetDb = (float) k.getValue(); engine.markModified(); };
        k.textFromValueFunction = [] (double v) { return juce::String (v, 1) + " dB"; };
        k.setPopupDisplayEnabled (true, true, nullptr);
        k.onLearn = [this, target] { control.armLearnFor (target); };
        l.setText (name, juce::dontSendNotification);
        l.setFont (Pequod::heading (10.5f));
        l.setColour (juce::Label::textColourId, Pequod::Colours::textDim);
        l.setJustificationType (juce::Justification::centredRight);
        addAndMakeVisible (k);
        addAndMakeVisible (l);
    };
    setupKnob (presetIn, inLabel, "PRESET IN", engine.getInputStage(), Mapping::Target::presetInGain);
    setupKnob (presetOut, outLabel, "PRESET OUT", engine.getOutputStage(), Mapping::Target::presetOutGain);
    presetIn.setTooltip ("This preset's input level (saved with the preset, and per scene). Double-click: 0 dB");
    presetOut.setTooltip ("This preset's volume (saved with the preset, and per scene). Right-click: match levels between presets");
    presetOut.addToMenu = [this] (juce::PopupMenu& m) { if (addOutMenuItems) addOutMenuItems (m); };

    rebuild();
    startTimerHz (15);
}

SceneBar::~SceneBar() { stopTimer(); }

void SceneBar::rebuild()
{
    sceneButtons.clear();
    builtNames.clear();
    for (int i = 0; i < engine.getNumScenes(); ++i)
    {
        auto* b = sceneButtons.add (new juce::TextButton (juce::String (i + 1) + "  " + engine.getSceneName (i)));
        styleButton (*b);
        b->setToggleState (i == engine.getCurrentScene(), juce::dontSendNotification);
        b->setTooltip ("Click: switch to this scene. Right-click: rename, move, learn a switch, delete");
        b->onClick = [this, i]
        {
            if (std::exchange (suppressClick, false)) return;    // that was a right-click (menu)
            engine.selectScene (i);
        };
        b->addMouseListener (this, false);
        addAndMakeVisible (b);
        builtNames.add (engine.getSceneName (i));
    }
    builtCount = engine.getNumScenes();
    builtCurrent = engine.getCurrentScene();
    resized();
    repaint();
}

void SceneBar::mouseDown (const juce::MouseEvent& e)
{
    if (! e.mods.isPopupMenu()) return;
    for (int i = 0; i < sceneButtons.size(); ++i)
        if (e.eventComponent == sceneButtons[i])
        {
            suppressClick = true;
            sceneMenu (i);
        }
}

void SceneBar::sceneMenu (int i)
{
    juce::PopupMenu m;
    m.addSectionHeader (engine.getSceneName (i));
    m.addItem ("Rename...", [this, i]
    {
        auto* w = new juce::AlertWindow ("Rename scene", {}, juce::MessageBoxIconType::NoIcon);
        w->addTextEditor ("name", engine.getSceneName (i));
        w->addButton ("OK", 1, juce::KeyPress (juce::KeyPress::returnKey));
        w->addButton ("Cancel", 0, juce::KeyPress (juce::KeyPress::escapeKey));
        w->enterModalState (true, juce::ModalCallbackFunction::create ([this, w, i] (int r)
        {
            if (r == 1) engine.renameScene (i, w->getTextEditorContents ("name"));
        }), true);
    });
    m.addItem ("Move left", i > 0, false, [this, i] { engine.moveScene (i, i - 1); });
    m.addItem ("Move right", i < engine.getNumScenes() - 1, false, [this, i] { engine.moveScene (i, i + 1); });
    m.addItem ("MIDI / OSC learn (a switch for this scene)", [this, i] { control.armLearnFor (Mapping::Target::sceneSelect, 0, i + 1); });
    m.addSeparator();
    m.addItem ("Delete this scene", [this, i] { engine.removeScene (i); });
    m.showMenuAsync (juce::PopupMenu::Options());
}

void SceneBar::showMeasuring (int secondsLeft)
{
    outLabel.setText (secondsLeft >= 0 ? "PLAY " + juce::String (secondsLeft) : juce::String ("PRESET OUT"), juce::dontSendNotification);
    outLabel.setColour (juce::Label::textColourId, secondsLeft >= 0 ? Pequod::Colours::mustard : Pequod::Colours::textDim);
}

void SceneBar::timerCallback()
{
    bool namesChanged = builtNames.size() != engine.getNumScenes();
    for (int i = 0; ! namesChanged && i < engine.getNumScenes(); ++i)
        namesChanged = builtNames[i] != engine.getSceneName (i);
    if (namesChanged || builtCount != engine.getNumScenes())
        rebuild();
    else if (builtCurrent != engine.getCurrentScene())
    {
        builtCurrent = engine.getCurrentScene();
        for (int i = 0; i < sceneButtons.size(); ++i)
            sceneButtons[i]->setToggleState (i == builtCurrent, juce::dontSendNotification);
    }

    if (! presetIn.isMouseButtonDown())  presetIn.setValue (engine.getInputStage()->presetDb.load(), juce::dontSendNotification);
    if (! presetOut.isMouseButtonDown()) presetOut.setValue (engine.getOutputStage()->presetDb.load(), juce::dontSendNotification);
    repaint (presetIn.getBounds().getUnion (presetOut.getBounds()).expanded (90, 0));
}

void SceneBar::paint (juce::Graphics& g)
{
    g.fillAll (Pequod::Colours::rail);
    g.setColour (Pequod::Colours::line);
    g.fillRect (getLocalBounds().removeFromBottom (1));

    auto r = getLocalBounds().reduced (12, 0);
    g.setColour (Pequod::Colours::mustard);
    g.setFont (Pequod::heading (13.0f));
    g.drawText ("SCENES", r.removeFromLeft (70), juce::Justification::centredLeft);

    if (engine.getNumScenes() == 0)
    {
        g.setColour (Pequod::Colours::textDim);
        g.setFont (Pequod::body (12.0f));
        g.drawText ("Versions of this preset, switched instantly (verse, chorus, solo...)",
                    juce::Rectangle<int> (addButton.getRight() + 12, 0, juce::jmax (0, inLabel.getX() - addButton.getRight() - 24), getHeight()),
                    juce::Justification::centredLeft, true);
    }

    // dB values under the knob labels
    g.setFont (Pequod::heading (10.0f, false));
    g.setColour (Pequod::Colours::text);
    auto valueText = [] (float db) { return (db > 0 ? "+" : "") + juce::String (db, 1) + " dB"; };
    g.drawText (valueText (engine.getInputStage()->presetDb.load()), inLabel.getBounds().translated (0, 14), juce::Justification::centredRight);
    g.drawText (valueText (engine.getOutputStage()->presetDb.load()), outLabel.getBounds().translated (0, 14), juce::Justification::centredRight);
}

void SceneBar::resized()
{
    auto r = getLocalBounds().reduced (12, 6);

    // levels on the right: [PRESET IN (knob)] [PRESET OUT (knob)]
    auto knobArea = [&] (LearnableSlider& k, juce::Label& l)
    {
        auto b = r.removeFromRight (150);
        k.setBounds (b.removeFromRight (36).withSizeKeepingCentre (36, 36));
        b.removeFromRight (6);
        l.setBounds (b.withTrimmedTop (2).withHeight (16));
    };
    knobArea (presetOut, outLabel);
    r.removeFromRight (6);
    knobArea (presetIn, inLabel);
    r.removeFromRight (16);

    r.removeFromLeft (70);
    const int w = sceneButtons.isEmpty() ? 0 : juce::jlimit (70, 150, (r.getWidth() - 96) / sceneButtons.size() - 4);
    for (auto* b : sceneButtons)
    {
        b->setBounds (r.removeFromLeft (w));
        r.removeFromLeft (4);
    }
    r.removeFromLeft (4);
    addButton.setBounds (r.removeFromLeft (88));
}
