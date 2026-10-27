#pragma once

#include "PequodNative.h"

//==============================================================================
/** The panel of a built-in PEQUOD plugin, drawn by the host from the layout the
    engine describes (the same layout as the VST3 version's panel): knobs,
    switches, selectors, meters, lamps, room lists and the preset bar. */
class PequodPanel : public juce::AudioProcessorEditor,
                    public juce::TooltipClient,
                    private juce::Timer
{
public:
    explicit PequodPanel (PequodNative::Instance&);
    ~PequodPanel() override;

    void paint (juce::Graphics&) override;
    void resized() override;
    void mouseDown (const juce::MouseEvent&) override;
    void mouseDrag (const juce::MouseEvent&) override;
    void mouseUp (const juce::MouseEvent&) override;
    void mouseDoubleClick (const juce::MouseEvent&) override;
    void mouseWheelMove (const juce::MouseEvent&, const juce::MouseWheelDetails&) override;
    void mouseMove (const juce::MouseEvent&) override;
    juce::String getTooltip() override;

private:
    enum class Kind { knob, sw, selector, meter, lamp, divider, text, radio, caption, roomplan, polar, irloader, tuner, rotor, unknown };

    struct Item
    {
        Kind kind = Kind::unknown;
        int p = -1, m = -1;            // parameter / meter index
        float x = 0, y = 0, r = 0, w = 0, h = 0, size = 12, lo = 0, hi = 1, rowH = 22;
        juce::String label, tip, meterKind;
        juce::StringArray labels;      // switch labels, selector positions, radio names, caption texts
        int activeP = -1;
        float activeMin = 0, activeMax = 0;
        int room = -1, distance = -1, spacing = -1, pattern = -1, angle = -1;
        int freq = -1, level = -1, reference = -1, horn = -1, drum = -1;
    };

    struct Preset { juce::String name; std::vector<std::pair<int, float>> values; };

    PequodNative::Instance& inst;
    juce::String title, sub1, sub2;
    juce::Colour accent { 0xff8e2a22 };
    int panelW = 640, panelH = 432;
    std::vector<Item> items;
    std::vector<Preset> presets;
    std::vector<float> meters, shownMeters;
    std::vector<float> lastValues;
    int currentPreset = -1;
    float rotorPhase[2] = { 0, 0 };

    // interaction
    int dragItem = -1;
    float dragStartValue = 0;
    juce::Point<float> dragStart, lastMouse;
    juce::TooltipWindow tooltips { this, 600 };

    juce::Rectangle<float> presetBox, prevBox, nextBox;

    juce::AudioProcessorParameter* paramObj (int p) const;
    float value (int p) const;
    int stepsOf (int p) const;
    int indexOf (int p) const;                  // stepped value as an index
    void setValue (int p, float normalised);
    void setIndex (int p, int index);
    bool isActive (const Item&) const;
    juce::String text (int p) const;
    int hitTest (juce::Point<float>) const;
    void applyPreset (int index);
    void showPresetMenu();

    void drawKnob (juce::Graphics&, const Item&) const;
    void drawSwitch (juce::Graphics&, const Item&) const;
    void drawSelector (juce::Graphics&, const Item&) const;
    void drawMeter (juce::Graphics&, const Item&) const;
    void drawLamp (juce::Graphics&, const Item&) const;
    void drawRadio (juce::Graphics&, const Item&) const;
    void drawCaption (juce::Graphics&, const Item&) const;
    void drawRoomPlan (juce::Graphics&, const Item&) const;
    void drawPolar (juce::Graphics&, const Item&) const;
    void drawTuner (juce::Graphics&, const Item&) const;
    void drawRotor (juce::Graphics&, const Item&) const;
    juce::Point<float> selectorLabelPos (const Item&, int position) const;

    void timerCallback() override;

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (PequodPanel)
};
