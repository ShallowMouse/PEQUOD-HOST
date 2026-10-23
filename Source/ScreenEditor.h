#pragma once

#include "ControllerManager.h"
#include "PequodLookAndFeel.h"

//==============================================================================
/** The SCREEN tab of the controller window: lay out what the controller's screen shows.
    Drag things around on the life-size preview, add text (with {tokens} PEQUOD fills in), lines and boxes. */
class ScreenEditor : public juce::Component,
                     private juce::Timer
{
public:
    explicit ScreenEditor (ControllerManager&);
    ~ScreenEditor() override;

    void paint (juce::Graphics&) override;
    void resized() override;
    bool keyPressed (const juce::KeyPress&) override;
    int getIdealHeight (int width) const;

    /** Re-reads the layout from the controller (when it connects or changes elsewhere). */
    void reload();
    void refreshPreview();
    std::function<void()> onSizeChanged;     // the list grew or shrank

private:
    ControllerManager& mgr;
    ScreenLayout::Layout layout;
    bool custom = false;
    int selected = -1;

    class Preview;
    std::unique_ptr<Preview> preview;

    juce::Label notice;
    juce::ToggleButton useToggle, barToggle;
    juce::TextButton addText { "+ TEXT" }, addLine { "+ LINE" }, addBox { "+ BOX" },
                     removeButton { "DELETE" }, upButton { "UP" }, downButton { "DOWN" },
                     standardButton { "START AGAIN FROM THE STANDARD SCREEN" };

    // the list of elements
    juce::OwnedArray<juce::TextButton> rows;

    // properties of the selected element
    juce::Label textLabel, sizeLabel, alignLabel, posLabel, boxLabel;
    juce::TextEditor textEditor;
    juce::TextButton insertButton { "INSERT..." };
    juce::OwnedArray<juce::TextButton> sizeButtons, alignButtons;
    juce::ToggleButton invertToggle, fitToggle;
    juce::Slider xSlider, ySlider, wSlider, hSlider;

    juce::Rectangle<int> previewArea, listArea, propsArea;

    ScreenLayout::Element* current();
    void select (int);
    void rebuildRows();
    void updateProperties();
    void changed (bool sendNow = true);
    void add (ScreenLayout::Element::Kind);
    void moveSelected (int delta);
    void nudge (int dx, int dy);
    void showTokenMenu();
    void timerCallback() override;

    friend class Preview;
};
