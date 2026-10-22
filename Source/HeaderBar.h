#pragma once

#include "ControlSurface.h"
#include "PequodLookAndFeel.h"
#include "Tuner.h"

//==============================================================================
/** Compact note + cents display fed by the Tuner. */
class TunerDisplay : public juce::Component,
                     public juce::SettableTooltipClient,
                     private juce::Timer
{
public:
    explicit TunerDisplay (Tuner& t) : tuner (t)
    {
        setTooltip ("Tuner - follows the guitar input. Right-click to change the reference pitch.");
        startTimerHz (30);
    }

    void paint (juce::Graphics&) override;
    void mouseDown (const juce::MouseEvent&) override;

private:
    Tuner& tuner;
    bool lastValid = false;
    float lastCents = 0.0f;
    int lastNote = -1;

    void timerCallback() override;
};

//==============================================================================
/** Tempo readout: shows the BPM and the beat. Drag up/down to change it, double-click to type it,
    click for the metronome click, its volume and the beats per bar. */
class TempoDisplay : public juce::Component,
                     public juce::SettableTooltipClient,
                     private juce::Timer
{
public:
    explicit TempoDisplay (Transport& t) : transport (t)
    {
        setTooltip ("Tempo for tempo-synced effects. Drag up/down to change it (shift = fine), double-click to type it, "
                    "click for the metronome click.");
        setMouseCursor (juce::MouseCursor::UpDownResizeCursor);
        startTimerHz (30);
    }

    std::function<void()> onChanged;      // tempo or click settings changed (to save them)

    void paint (juce::Graphics&) override;
    void mouseDown (const juce::MouseEvent&) override;
    void mouseDrag (const juce::MouseEvent&) override;
    void mouseUp (const juce::MouseEvent&) override;
    void mouseDoubleClick (const juce::MouseEvent&) override;

private:
    Transport& transport;
    double dragStartBpm = 120.0;
    bool dragged = false;
    int lastBeat = -1;
    double lastBpm = 0;
    bool lastClick = false;

    void timerCallback() override;
    void showMenu();
    void typeTempo();
};

//==============================================================================
/** Top strip: wordmark, preset navigation, learn, tuner + mute, settings, master IN/OUT with meters. */
class HeaderBar : public juce::Component,
                  private juce::Timer
{
public:
    HeaderBar (HostEngine&, ControlSurface&, Tuner&);
    ~HeaderBar() override;

    void setPresetName (const juce::String& name, bool modified);
    /** Shows the UPDATE button (empty version hides it). */
    void setUpdate (const juce::String& version, bool downloading, float progress);
    std::function<void()> onUpdate;

    std::function<void()> onPrev, onNext, onSave, onSaveAs, onInit, onAudioSettings, onPluginManager, onToggleLearn;
    std::function<void()> onPresetMenu, onController;
    std::function<int()> controllerState;   // 0 none, 1 connected, 2 updating
    std::function<void (int stages)> onPipelineStagesChanged;
    std::function<void()> onTempoSettingsChanged;

    void paint (juce::Graphics&) override;
    void resized() override;

private:
    HostEngine& engine;
    ControlSurface& control;

    juce::TextButton prevButton { "<" }, nextButton { ">" }, presetButton,
                     saveButton { "SAVE" }, saveAsButton { "SAVE AS" }, initButton { "INIT" },
                     learnButton { "LEARN" }, audioButton { "AUDIO" }, pluginsButton { "PLUGINS" };

    /** Opens the foot controller window; the dot shows whether one is connected. */
    struct ControllerButton : public juce::TextButton
    {
        ControllerButton() : TextButton ("CONTROLLER") {}
        int state = 0;
        void paintButton (juce::Graphics&, bool over, bool down) override;
    };
    ControllerButton controllerButton;
    juce::TextButton updateButton;

    TunerDisplay tunerDisplay;
    TempoDisplay tempoDisplay;
    LearnableButton tapButton { "TAP" };
    LearnableButton muteButton { "MUTE" };

    LearnableSlider inKnob, outKnob;
    juce::Label inLabel, outLabel;

    /** CPU meter that doubles as the multi-core setting. */
    struct CpuButton : public juce::Button
    {
        CpuButton() : Button ("cpu") {}
        juce::String top, bottom;
        bool hot = false;
        void paintButton (juce::Graphics&, bool over, bool down) override;
    };
    CpuButton cpuButton;
    void showCoresMenu();
    PeakMeter inMeter, outMeter;

    juce::Rectangle<int> wordmarkArea;

    void timerCallback() override;
    void setupKnob (LearnableSlider&, juce::Label&, const juce::String&, GainMeterProcessor*, Mapping::Target);
};
