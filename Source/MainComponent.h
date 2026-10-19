#pragma once

#include "HostEngine.h"
#include "ControlSurface.h"
#include "HeaderBar.h"
#include "BrowserPanel.h"
#include "RackView.h"
#include "ControlPanel.h"

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

    HostEngine engine;
    ControlSurface control { engine };
    Tuner tuner { engine };

    HeaderBar header { engine, control, tuner };
    BrowserPanel browser { engine, presetDir };
    RackView rack;
    ControlPanel controlPanel { engine, control };
    juce::Label statusBar;
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
    std::unique_ptr<juce::XmlElement> createFullRackXml() const;
    juce::String getPresetDisplayName() const;

    //== Windows / dialogs
    void openPluginWindow (int slotUid);
    void closePluginWindow (int slotUid);
    void showAudioSettings();
    void showPluginManager (bool autoScan);
    void addPluginAtEnd (const juce::PluginDescription&);

    void setStatus (const juce::String&);

    void changeListenerCallback (juce::ChangeBroadcaster*) override;
    void timerCallback() override;

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (MainComponent)
};
