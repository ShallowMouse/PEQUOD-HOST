#pragma once

#include "ControllerManager.h"
#include "PequodLookAndFeel.h"

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

    juce::TextButton controlsTab { "CONTROLS" }, settingsTab { "SETTINGS" }, quickSetupButton { "QUICK SETUP" };
    juce::OwnedArray<juce::TextButton> layerButtons;
    std::unique_ptr<BoardView> board;
    std::unique_ptr<Inspector> inspector;
    std::unique_ptr<SettingsPage> settings;
    juce::Viewport settingsViewport;

    // shown when nothing is connected
    juce::TextEditor ipEditor;
    juce::TextButton ipButton { "CONNECT" };

    bool showingSettings = false;
    bool wasConnected = false;
    int idleTicks = 0;
    juce::String lastSignature;

    void showSettings (bool);
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
