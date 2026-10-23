#pragma once

#include "ControllerManager.h"
#include "PequodLookAndFeel.h"
#include "ScreenEditor.h"

//==============================================================================
/** The CONTROLLER window: the foot controller drawn as it is on the floor. Press a
    switch (on the screen or with your foot) to pick it, then choose what it does.
    SETTINGS holds everything else the controller stores. */
class ControllerPanel : public juce::Component,
                        private juce::ChangeListener,
                        private juce::Timer
{
public:
    ControllerPanel (ControllerManager&, HostEngine&, ControlSurface&);
    ~ControllerPanel() override;

    void paint (juce::Graphics&) override;
    void resized() override;

    void select (const juce::String& uid, int code);

    class BoardView;
    class Inspector;
    class SettingsPage;

private:
    ControllerManager& mgr;
    HostEngine& engine;
    ControlSurface& control;

    juce::TextButton controlsTab { "CONTROLS" }, screenTab { "SCREEN" }, settingsTab { "SETTINGS" };
    juce::OwnedArray<juce::TextButton> layerButtons;
    std::unique_ptr<BoardView> board;
    std::unique_ptr<Inspector> inspector;
    std::unique_ptr<SettingsPage> settings;
    std::unique_ptr<ScreenEditor> screenEditor;
    /** Scrolls sideways only; an ordinary mouse wheel scrolls the page around it instead. */
    struct SideScroller : public juce::Viewport
    {
        void mouseWheelMove (const juce::MouseEvent& e, const juce::MouseWheelDetails& w) override
        {
            if (w.deltaX != 0.0f || e.mods.isShiftDown()) { Viewport::mouseWheelMove (e, w); return; }
            if (auto* p = getParentComponent()) p->mouseWheelMove (e.getEventRelativeTo (p), w);
        }
    };

    juce::Component controlsPage;      // declared before the viewport that shows it
    juce::Viewport settingsViewport, controlsViewport, screenViewport;
    SideScroller boardViewport;

    // shown when nothing is connected
    juce::TextEditor ipEditor;
    juce::TextButton ipButton { "CONNECT" };

    bool showingSettings = false, showingScreen = false;
    bool wasConnected = false;
    int idleTicks = 0;
    juce::Rectangle<int> barArea;     // layer buttons + which preset is being edited
    bool inResized = false;
    juce::String lastSignature;

    void showSettings (bool);
    void showScreen();
    void updateVisibility();
    void changeListenerCallback (juce::ChangeBroadcaster*) override;
    void timerCallback() override;
};

//==============================================================================
class ControllerWindow : public juce::DocumentWindow
{
public:
    ControllerWindow (ControllerManager&, HostEngine&, ControlSurface&);
    void closeButtonPressed() override { setVisible (false); }
    ControllerPanel& getPanel() { return *panel; }

private:
    ControllerPanel* panel = nullptr;
};
