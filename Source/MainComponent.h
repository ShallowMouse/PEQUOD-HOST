#pragma once
#include <set>
#include "CrashGuard.h"

#include "HostEngine.h"
#include "ControlSurface.h"
#include "HeaderBar.h"
#include "BrowserPanel.h"
#include "RackView.h"
#include "ControlPanel.h"
#include "PresetLibrary.h"
#include "ControllerManager.h"
#include "Updater.h"

class ControllerWindow;

class PluginWindow;

//==============================================================================
class MainComponent : public juce::Component,
                      public juce::DragAndDropContainer,
                      private juce::ChangeListener,
                      private juce::Timer
{
public:
    MainComponent();
    ~MainComponent() override;

    void paint (juce::Graphics&) override;
    void resized() override;
    bool keyPressed (const juce::KeyPress&) override;

    void saveSettings();

private:
    std::unique_ptr<juce::ApplicationProperties> appProperties;
    juce::File presetDir;
    PresetLibrary library { presetDir };

    HostEngine engine;
    ControlSurface control { engine };
    Tuner tuner { engine };
    std::unique_ptr<ControllerManager> controller;
    std::unique_ptr<ControllerWindow> controllerWindow;
    Updater updater;

    HeaderBar header { engine, control, tuner };
    BrowserPanel browser { engine, library };
    RackView rack;
    ControlPanel controlPanel { engine, control };
    juce::Label statusBar;
    juce::TextButton rackViewButton { "RACK" }, pedalboardViewButton { "PEDALBOARD" };
    juce::TextButton undoButton { "UNDO" }, redoButton { "REDO" };

    // On-screen keyboard for instrument plugins (also played from the computer keyboard: A W S E D...)
    juce::TextButton keysButton { "KEYS" };
    std::unique_ptr<juce::MidiKeyboardComponent> keyboard;
    void showKeyboard (bool);

    // Undo / redo of rack edits: whole-rack snapshots (plugins, settings, mappings).
    std::vector<std::unique_ptr<juce::XmlElement>> undoStack, redoStack;
    bool restoringUndo = false;
    void pushUndo();
    void clearUndo();
    void undo();
    void redo();
    void restoreSnapshot (const juce::XmlElement&);
    void updateUndoButtons();

    std::set<const PluginSlot*> reportedCrashes;
    bool waitingForCrashAnswer = false;
    void askAfterCrash (const CrashGuard::LastRun&);
    void setPedalboardView (bool);
    juce::TooltipWindow tooltips { this, 600 };

    std::map<int, std::unique_ptr<PluginWindow>> windows;

    juce::File currentPresetFile;
    int savedChangeCounter = 0;
    juce::String statusMessage;
    double statusTime = 0;
    int autosaveTicks = 0;

    //== Presets
    juce::Array<juce::File> getPresetFiles() const;
    void loadRack (const juce::XmlElement&);
    void loadPresetFile (const juce::File&);
    void loadPresetIndex (int index);
    void loadPresetByName (const juce::String&);
    void stepPreset (int delta);
    void savePreset (const juce::File&);
    void save();
    void saveAs();
    void initRack();
    void deletePreset (const juce::File&);
    void showPresetMenu();

    //== Banks
    void stepBank (int delta);
    void selectBank (int index, bool loadPresetToo);
    void newBank();
    void renameBank (const juce::String&);
    void deleteBank (const juce::String&);
    void movePreset (const juce::File&, const juce::String& bank);
    void askForText (const juce::String& title, const juce::String& label, const juce::String& initial,
                     std::function<void (const juce::String&)> onOk);
    std::unique_ptr<juce::XmlElement> createFullRackXml() const;
    juce::String getPresetDisplayName() const;

    //== Windows / dialogs
    void openPluginWindow (int slotUid);
    void closePluginWindow (int slotUid);
    void showAudioSettings();
    void showPluginManager (bool autoScan);
    void showController();
    void addPluginAtEnd (const juce::PluginDescription&);

    void setStatus (const juce::String&);

    void changeListenerCallback (juce::ChangeBroadcaster*) override;
    void timerCallback() override;

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (MainComponent)
};
