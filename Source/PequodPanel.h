#pragma once

#include "PequodNative.h"

//==============================================================================
/** The panel of a built-in PEQUOD plugin, drawn by the host from the layout the
    engine describes. It is a straight port of the VST3 panel (vintage/src/ui.rs
    in engine/): same faceplate, knobs, switches, selectors, meters, lamps, room
    plans, polar plots, tuner, rotor, IR loader and preset bar, same sizes,
    colours and fonts, and the same mouse handling. */
class PequodPanel : public juce::AudioProcessorEditor,
                    public juce::TooltipClient,
                    private juce::Timer
{
public:
    explicit PequodPanel (PequodNative::Instance&);
    ~PequodPanel() override;

    void paint (juce::Graphics&) override;
    void mouseDown (const juce::MouseEvent&) override;
    void mouseDrag (const juce::MouseEvent&) override;
    void mouseUp (const juce::MouseEvent&) override;
    void mouseDoubleClick (const juce::MouseEvent&) override;
    void mouseWheelMove (const juce::MouseEvent&, const juce::MouseWheelDetails&) override;
    void mouseMove (const juce::MouseEvent&) override;
    void mouseExit (const juce::MouseEvent&) override;
    juce::String getTooltip() override;

private:
    enum class Kind { knob, sw, selector, meter, lamp, divider, text, radio, caption, roomplan, polar, irloader, tuner, rotor, unknown };

    struct Plan { float length = 10, width = 6, srcX = 2, srcY = 3; bool openAir = false; juce::String farWall; };

    struct Item
    {
        Kind kind = Kind::unknown;
        int p = -1, m = -1;
        float x = 0, y = 0, r = 20, w = 0, h = 0, size = 12, lo = 0, hi = 1, rowH = 22;
        int alignX = 0, alignY = 0;
        juce::String label, tip, meterKind;
        juce::StringArray labels;
        int activeP = -1;
        float activeMin = 0, activeMax = 1;
        int room = -1, distance = -1, spacing = -1, pattern = -1, angle = -1;
        int freq = -1, level = -1, reference = -1, horn = -1, drum = -1, cab = -1, irPos = -1;
        std::vector<Plan> plans;
    };

    struct Preset { juce::String name; std::vector<std::pair<int, float>> values; };

    PequodNative::Instance& inst;
    juce::String title, sub1, sub2;
    juce::Colour accent { 0xff8e2a22 };
    int panelW = 640, panelH = 432;
    std::vector<Item> items;
    std::vector<Preset> presets;

    // meters, with the VST3 panel's ballistics
    std::vector<float> raw, shown;
    float tunerLevel = 0;
    double tunerSeen = -1.0e9, lastTick = 0;
    std::vector<float> lastValues;

    // presets
    int currentPreset = -1;
    bool presetDirty = false, presetOpen = false;
    int presetHover = -1;

    // interaction
    int hoverItem = -1, dragItem = -1, hoverButton = 0;   // hoverButton: 1 prev, 2 next, 3 name, 4 IR load, 5 IR clear
    float dragStartValue = 0, selectorAcc = 0;
    juce::Point<float> dragStart, lastDrag, lastMouse;
    bool dragged = false;
    juce::TooltipWindow tooltips { this, 600 };
    std::unique_ptr<juce::FileChooser> chooser;

    juce::AudioProcessorParameter* paramObj (int p) const;
    float value (int p) const;
    float plain (int p) const;
    int stepsOf (int p) const;
    int indexOf (int p) const;
    void setValue (int p, float normalised, bool gesture);
    void setIndex (int p, int index);
    bool isDim (const Item&) const;
    juce::String text (int p) const;
    int hitTest (juce::Point<float>) const;
    void applyPreset (int index);
    void edited();

    juce::Point<float> presetCentre() const;
    juce::Rectangle<float> presetNameRect() const;
    juce::Rectangle<float> presetPrevRect() const;
    juce::Rectangle<float> presetNextRect() const;
    juce::Rectangle<float> presetListRect() const;
    juce::Rectangle<float> presetRowRect (int) const;
    const Item* irItem() const;
    juce::Rectangle<float> irButton (const Item&) const;
    juce::Rectangle<float> irClear (const Item&) const;

    void drawFaceplate (juce::Graphics&) const;
    void drawKnob (juce::Graphics&, const Item&, int index) const;
    void drawSwitch (juce::Graphics&, const Item&, int index) const;
    void drawSelector (juce::Graphics&, const Item&, int index) const;
    void drawMeter (juce::Graphics&, const Item&) const;
    void drawLamp (juce::Graphics&, const Item&) const;
    void drawRadio (juce::Graphics&, const Item&) const;
    void drawCaption (juce::Graphics&, const Item&) const;
    void drawRoomPlan (juce::Graphics&, const Item&) const;
    void drawPolar (juce::Graphics&, const Item&) const;
    void drawTuner (juce::Graphics&, const Item&) const;
    void drawRotor (juce::Graphics&, const Item&) const;
    void drawIrLoader (juce::Graphics&, const Item&) const;
    void drawPresetBar (juce::Graphics&) const;
    void drawPresetList (juce::Graphics&) const;
    float selectorAngle (const Item&, int position) const;

    void timerCallback() override;

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (PequodPanel)
};
