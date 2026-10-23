#include "MainComponent.h"
#include "SettingsPanel.h"
#include "PluginManagerPanel.h"
#include "ControllerPanel.h"
#include "CrashGuard.h"

//==============================================================================
class PluginWindow : public juce::DocumentWindow
{
public:
    PluginWindow (const juce::String& title, juce::AudioProcessorEditor* editor, std::function<void()> onCloseFn)
        : DocumentWindow (title, Pequod::Colours::bg, juce::DocumentWindow::closeButton | juce::DocumentWindow::minimiseButton),
          onClose (std::move (onCloseFn))
    {
        setUsingNativeTitleBar (true);
        setContentOwned (editor, true);
        setResizable (editor->isResizable(), false);
        centreAroundComponent (nullptr, getWidth(), getHeight());
        setVisible (true);
    }

    ~PluginWindow() override { clearContentComponent(); }

    void closeButtonPressed() override { if (onClose) onClose(); }

private:
    std::function<void()> onClose;
};

//==============================================================================
static std::unique_ptr<juce::ApplicationProperties> makeProperties()
{
    juce::PropertiesFile::Options o;
    o.applicationName     = "PEQUOD Host";
    o.folderName          = "PEQUOD";
    o.filenameSuffix      = "settings";
    o.osxLibrarySubFolder = "Application Support";
    o.storageFormat       = juce::PropertiesFile::storeAsXML;

    auto p = std::make_unique<juce::ApplicationProperties>();
    p->setStorageParameters (o);
    return p;
}

MainComponent::MainComponent()
    : appProperties (makeProperties()),
      presetDir (juce::File::getSpecialLocation (juce::File::userDocumentsDirectory).getChildFile ("PEQUOD Host").getChildFile ("Presets")),
      rack (RackContext { engine, control,
                          [this] (int uid) { openPluginWindow (uid); },
                          [this] (int uid) { return windows.count (uid) > 0; },
                          [this] (int uid) { closePluginWindow (uid); },
                          [this] (const juce::String& s) { setStatus (s); } })
{
    auto* settings = appProperties->getUserSettings();
    const auto lastRun = CrashGuard::start (settings->getFile().getParentDirectory());

    // Plugins
    if (auto xml = settings->getXmlValue ("knownPlugins"))
        engine.knownPlugins.recreateFromXml (*xml);
    engine.knownPlugins.addChangeListener (this);

    // Audio + MIDI
    auto deviceXml = settings->getXmlValue ("audioDevice");
    engine.initialiseAudio (deviceXml.get());

    engine.setPipelineStages (settings->getIntValue ("pipelineStages", 1));
    engine.transport.clickOn = settings->getBoolValue ("clickOn", false);
    engine.transport.clickGainDb = (float) settings->getDoubleValue ("clickDb", -12.0);
    engine.transport.beatsPerBar = juce::jlimit (2, 7, settings->getIntValue ("beatsPerBar", 4));
    engine.getOutputStage()->limiterOn = settings->getBoolValue ("limiter", true);
    if (settings->containsKey ("loudnessTarget"))
        engine.loudnessReference = settings->getDoubleValue ("loudnessTarget");

    // MIDI / OSC
    auto controlXml = settings->getXmlValue ("control");
    control.loadGlobalXml (controlXml.get());

    control.onPresetNext  = [this] { stepPreset (1); };
    control.onPresetPrev  = [this] { stepPreset (-1); };
    control.onPresetIndex = [this] (int i) { loadPresetIndex (i); };
    control.onPresetName  = [this] (const juce::String& n) { loadPresetByName (n); };
    control.getPresetName = [this] { return getPresetDisplayName(); };
    control.getPresetIndex = [this] { return getPresetFiles().indexOf (currentPresetFile); };
    control.onBankNext     = [this] { stepBank (1); };
    control.onBankPrev     = [this] { stepBank (-1); };
    control.onBankIndex    = [this] (int i, bool loadToo) { selectBank (i, loadToo); };
    control.onBankName     = [this] (const juce::String& n) { selectBank (library.getBanks().indexOf (n), true); };
    control.getBankName    = [this] { return library.getCurrentBank(); };
    control.getBankIndex   = [this] { return library.getCurrentBankIndex(); };
    control.addChangeListener (this);
    control.setTuner (&tuner);
    control.onGlobalSettingsChanged = [this]
    {
        juce::Component::SafePointer<MainComponent> safe (this);
        juce::MessageManager::callAsync ([safe] { if (safe != nullptr) safe->saveSettings(); });
    };

    // Foot controller (PEQUOD FC)
    controller = std::make_unique<ControllerManager> (engine, control, tuner, *settings);
    controller->onStatus = [this] (const juce::String& s) { setStatus (s); };
    controller->getPresetNames = [this]
    {
        juce::StringArray names;
        for (auto& f : getPresetFiles()) names.add (f.getFileNameWithoutExtension());
        return names;
    };
    header.onController = [this] { showController(); };
    header.onStatus = [this] (const juce::String& t) { setStatus (t); };
    header.onLevelTargetChanged = [this] { saveSettings(); };
    engine.onBeforeEdit = [this] { pushUndo(); };
    for (auto* b : { &undoButton, &redoButton })
    {
        b->setColour (juce::TextButton::buttonColourId, Pequod::Colours::panel);
        addAndMakeVisible (b);
    }
    keyboard = std::make_unique<juce::MidiKeyboardComponent> (engine.getRack().keyboardState,
                                                              juce::MidiKeyboardComponent::horizontalKeyboard);
    keyboard->setAvailableRange (24, 108);
    keyboard->setLowestVisibleKey (48);
    keyboard->setKeyPressBaseOctave (4);
    keyboard->setOctaveForMiddleC (4);
    keyboard->setColour (juce::MidiKeyboardComponent::whiteNoteColourId, juce::Colour (0xffe8e2d4));
    keyboard->setColour (juce::MidiKeyboardComponent::blackNoteColourId, juce::Colour (0xff1c1c1c));
    keyboard->setColour (juce::MidiKeyboardComponent::keySeparatorLineColourId, juce::Colour (0xff4a4a4a));
    keyboard->setColour (juce::MidiKeyboardComponent::keyDownOverlayColourId, Pequod::Colours::mustard);
    keyboard->setColour (juce::MidiKeyboardComponent::mouseOverKeyOverlayColourId, Pequod::Colours::mustard.withAlpha (0.3f));
    keyboard->setColour (juce::MidiKeyboardComponent::shadowColourId, juce::Colours::black.withAlpha (0.4f));
    keyboard->setColour (juce::MidiKeyboardComponent::upDownButtonBackgroundColourId, Pequod::Colours::panel);
    keyboard->setColour (juce::MidiKeyboardComponent::upDownButtonArrowColourId, Pequod::Colours::mustard);
    keyboard->setColour (juce::MidiKeyboardComponent::textLabelColourId, juce::Colour (0xff555555));
    addChildComponent (*keyboard);
    keysButton.setColour (juce::TextButton::buttonColourId, Pequod::Colours::panel);
    keysButton.setColour (juce::TextButton::buttonOnColourId, Pequod::Colours::mustard);
    keysButton.setColour (juce::TextButton::textColourOnId, Pequod::Colours::rail);
    keysButton.setTooltip ("On-screen keyboard for instrument plugins (click it, then play with A W S E D F T G... too)");
    keysButton.onClick = [this] { showKeyboard (! keyboard->isVisible()); };
    addAndMakeVisible (keysButton);
    engine.getRack().muteGuitarForInstruments = settings->getBoolValue ("muteGuitarForInstruments", true);
    showKeyboard (settings->getBoolValue ("keyboardShown", false));

    undoButton.setTooltip ("Undo the last rack change (Ctrl+Z)");
    redoButton.setTooltip ("Redo (Ctrl+Y)");
    undoButton.onClick = [this] { undo(); };
    redoButton.onClick = [this] { redo(); };
    updateUndoButtons();
    header.onTempoSettingsChanged = [this] { engine.markModified(); };     // the tempo is saved with the preset

    // Updates: checked in the background every time PEQUOD starts
    updater.onChange = [this]
    {
        header.setUpdate (updater.isUpdateAvailable() ? updater.getLatestVersion() : juce::String(),
                          updater.isDownloading(), updater.getProgress());
    };
    updater.onStatus = [this] (const juce::String& s) { setStatus (s); };
    header.onUpdate = [this]
    {
        auto notes = updater.getReleaseNotes();
        juce::AlertWindow::showOkCancelBox (juce::MessageBoxIconType::NoIcon, "PEQUOD Host " + updater.getLatestVersion(),
            notes.isNotEmpty() ? "What's new:\n\n" + notes : juce::String ("A new version is ready."),
            "Install", "Later", nullptr,
            juce::ModalCallbackFunction::create ([this] (int r)
            {
                if (r != 1) return;
                saveSettings();
                updater.install();
                setStatus ("Downloading PEQUOD Host " + updater.getLatestVersion() + "...");
            }));
    };
   #if JUCE_WINDOWS
    updater.checkSoon();
   #endif
    header.controllerState = [this] { return controller->isUpdating() ? 2 : controller->isConnected() ? 1 : 0; };

    engine.onSlotAboutToBeRemoved = [this] (int uid)
    {
        windows.erase (uid);
        rack.dropEditor (uid);
        control.slotRemoved (uid);
    };

    // Header
    header.onPrev          = [this] { stepPreset (-1); };
    header.onNext          = [this] { stepPreset (1); };
    header.onPresetMenu    = [this] { showPresetMenu(); };
    header.onSave          = [this] { save(); };
    header.onSaveAs        = [this] { saveAs(); };
    header.onInit          = [this] { initRack(); };
    header.onAudioSettings = [this] { showAudioSettings(); };
    header.onPluginManager = [this] { showPluginManager (false); };
    header.onPipelineStagesChanged = [this] (int n)
    {
        engine.setPipelineStages (n);
        setStatus (n == 1 ? juce::String ("Running the chain on 1 core")
                          : "Chain spread over " + juce::String (n) + " cores");
    };
    header.onToggleLearn   = [this]
    {
        if (control.getLearnState() != ControlSurface::LearnState::off) control.cancelLearn();
        else control.armLearn();
    };

    // Browser
    browser.onAddPlugin    = [this] (const juce::PluginDescription& d) { addPluginAtEnd (d); };
    browser.onLoadPreset   = [this] (const juce::File& f) { loadPresetFile (f); };
    browser.onDeletePreset = [this] (const juce::File& f) { deletePreset (f); };
    browser.onNewBank      = [this] { newBank(); };
    browser.onRenameBank   = [this] (const juce::String& b) { renameBank (b); };
    browser.onDeleteBank   = [this] (const juce::String& b) { deleteBank (b); };
    browser.onMovePreset   = [this] (const juce::File& f, const juce::String& b) { movePreset (f, b); };
    browser.setGrouping (settings->getIntValue ("browserGrouping", 0) == 1 ? BrowserPanel::Grouping::collection
                                                                          : BrowserPanel::Grouping::function);
    browser.onGroupingChanged = [this] (BrowserPanel::Grouping g)
    {
        appProperties->getUserSettings()->setValue ("browserGrouping", g == BrowserPanel::Grouping::collection ? 1 : 0);
    };
    library.addChangeListener (this);

    browser.refresh();

    controlPanel.onShowInRack = [this] (int uid) { rack.showModule (uid); };

    statusBar.setFont (Pequod::heading (12.0f, false));
    statusBar.setColour (juce::Label::textColourId, Pequod::Colours::textDim);

    for (auto* c : std::initializer_list<juce::Component*> { &header, &browser, &rack, &controlPanel, &statusBar,
                                                            &rackViewButton, &pedalboardViewButton })
        addAndMakeVisible (c);

    rackViewButton.setTooltip ("Vertical rack");
    pedalboardViewButton.setTooltip ("Horizontal pedalboard");
    for (auto* b : { &rackViewButton, &pedalboardViewButton })
        b->setColour (juce::TextButton::buttonColourId, Pequod::Colours::panel);
    rackViewButton.onClick       = [this] { setPedalboardView (false); };
    pedalboardViewButton.onClick = [this] { setPedalboardView (true); };
    setPedalboardView (settings->getBoolValue ("pedalboardView", false));

    // Restore last session
    currentPresetFile = juce::File (settings->getValue ("lastPreset"));
    if (! currentPresetFile.existsAsFile())   // saved before banks: it now lives in "Default"
    {
        auto moved = library.getBankFolder ("Default").getChildFile (currentPresetFile.getFileName());
        currentPresetFile = moved.existsAsFile() ? moved : juce::File();
    }
    library.setCurrentBank (currentPresetFile.existsAsFile() ? PresetLibrary::bankOf (currentPresetFile)
                                                             : settings->getValue ("currentBank"));
    if (lastRun.crashed)
    {
        // PEQUOD didn't close properly last time: ask before loading the same plugins again.
        waitingForCrashAnswer = true;
        juce::MessageManager::callAsync ([this, lastRun] { askAfterCrash (lastRun); });
    }
    else
    {
        if (auto session = settings->getXmlValue ("session"))
            loadRack (*session);
        if (! settings->getBoolValue ("sessionModified", false))
            savedChangeCounter = engine.getChangeCounter();
    }
    browser.setCurrentPresetFile (currentPresetFile);

    if (engine.knownPlugins.getNumTypes() == 0)
        juce::MessageManager::callAsync ([this] { showPluginManager (true); });

    setWantsKeyboardFocus (true);
    setSize (1440, 900);
    startTimerHz (5);
    setStatus ("Ready. OSC in :" + juce::String (control.getOscInputPort())
               + (control.isOscInConnected() ? "" : " (port busy)"));
}

MainComponent::~MainComponent()
{
    stopTimer();
    control.onGlobalSettingsChanged = nullptr;
    controllerWindow.reset();
    saveSettings();

    windows.clear();
    rack.collapseAll();
    engine.onSlotAboutToBeRemoved = [this] (int uid) { rack.dropEditor (uid); };
    engine.clearRack();
    engine.onSlotAboutToBeRemoved = nullptr;

    library.removeChangeListener (this);
    controller.reset();
    control.setTuner (nullptr);
    control.removeChangeListener (this);
    engine.knownPlugins.removeChangeListener (this);
    CrashGuard::cleanExit();
}

void MainComponent::saveSettings()
{
    auto* s = appProperties->getUserSettings();

    if (auto dev = engine.deviceManager.createStateXml())
        s->setValue ("audioDevice", dev.get());
    if (auto plugins = engine.knownPlugins.createXml())
        s->setValue ("knownPlugins", plugins.get());
    if (auto c = control.createGlobalXml())
        s->setValue ("control", c.get());
    if (! waitingForCrashAnswer)        // don't overwrite the session we're asking about
        if (auto rackXml = createFullRackXml())
            s->setValue ("session", rackXml.get());

    s->setValue ("lastPreset", currentPresetFile.getFullPathName());
    s->setValue ("currentBank", library.getCurrentBank());
    s->setValue ("pipelineStages", engine.getPipelineStages());
    s->setValue ("clickOn", engine.transport.clickOn.load());
    s->setValue ("clickDb", (double) engine.transport.clickGainDb.load());
    s->setValue ("beatsPerBar", engine.transport.beatsPerBar.load());
    s->setValue ("limiter", engine.getOutputStage()->limiterOn.load());
    s->setValue ("muteGuitarForInstruments", engine.getRack().muteGuitarForInstruments.load());
    s->setValue ("keyboardShown", keyboard != nullptr && keyboard->isVisible());
    if (! std::isnan (engine.loudnessReference)) s->setValue ("loudnessTarget", engine.loudnessReference);
    s->setValue ("pedalboardView", rack.isPedalboard());
    s->setValue ("sessionModified", engine.getChangeCounter() != savedChangeCounter);
    s->saveIfNeeded();
}

//==============================================================================
void MainComponent::paint (juce::Graphics& g)
{
    g.fillAll (Pequod::Colours::bg);
    auto sb = statusBar.getBounds();
    g.setColour (juce::Colour (0xff141414));
    g.fillRect (sb.withX (0).withWidth (getWidth()));
    g.setColour (Pequod::Colours::line);
    g.drawHorizontalLine (sb.getY(), 0.0f, (float) getWidth());
}

void MainComponent::resized()
{
    auto r = getLocalBounds();
    header.setBounds (r.removeFromTop (64));
    auto status = r.removeFromBottom (24);
    auto toggles = status.removeFromRight (380).reduced (4, 3);
    pedalboardViewButton.setBounds (toggles.removeFromRight (100));
    toggles.removeFromRight (4);
    rackViewButton.setBounds (toggles.removeFromRight (60));
    toggles.removeFromRight (16);
    redoButton.setBounds (toggles.removeFromRight (56));
    toggles.removeFromRight (4);
    undoButton.setBounds (toggles.removeFromRight (56));
    toggles.removeFromRight (16);
    keysButton.setBounds (toggles.removeFromRight (56));
    statusBar.setBounds (status.reduced (10, 0));
    browser.setBounds (r.removeFromLeft (260));
    controlPanel.setBounds (r.removeFromRight (280));
    if (keyboard != nullptr && keyboard->isVisible())
    {
        auto k = r.removeFromBottom (96).reduced (8, 6);
        keyboard->setBounds (k);
        keyboard->setKeyWidth (juce::jlimit (16.0f, 34.0f, (float) k.getWidth() / 36.0f));   // about three octaves
    }
    rack.setBounds (r);
}

bool MainComponent::keyPressed (const juce::KeyPress& k)
{
    if (k == juce::KeyPress ('s', juce::ModifierKeys::commandModifier, 0)) { save(); return true; }
    if (k == juce::KeyPress ('s', juce::ModifierKeys::commandModifier | juce::ModifierKeys::shiftModifier, 0)) { saveAs(); return true; }
    if (k == juce::KeyPress ('z', juce::ModifierKeys::commandModifier, 0)) { undo(); return true; }
    if (k == juce::KeyPress ('y', juce::ModifierKeys::commandModifier, 0)
         || k == juce::KeyPress ('z', juce::ModifierKeys::commandModifier | juce::ModifierKeys::shiftModifier, 0)) { redo(); return true; }
    if (k == juce::KeyPress::escapeKey && control.getLearnState() != ControlSurface::LearnState::off) { control.cancelLearn(); return true; }
    return false;
}

void MainComponent::setPedalboardView (bool horizontal)
{
    rack.setPedalboard (horizontal);
    rackViewButton.setToggleState (! horizontal, juce::dontSendNotification);
    pedalboardViewButton.setToggleState (horizontal, juce::dontSendNotification);
}

void MainComponent::setStatus (const juce::String& s)
{
    statusMessage = s;
    statusTime = juce::Time::getMillisecondCounterHiRes();
    timerCallback();
}

void MainComponent::timerCallback()
{
    auto learn = control.getLearnStatusText();
    if (learn.isNotEmpty())
    {
        statusBar.setColour (juce::Label::textColourId, Pequod::Colours::mustard);
        statusBar.setText (learn + "   (Esc to cancel)", juce::dontSendNotification);
    }
    else
    {
        statusBar.setColour (juce::Label::textColourId, Pequod::Colours::textDim);
        auto old = juce::Time::getMillisecondCounterHiRes() - statusTime > 8000.0;
        statusBar.setText (old ? juce::String() : statusMessage, juce::dontSendNotification);
    }

    header.setPresetName ((currentPresetFile.existsAsFile() ? PresetLibrary::bankOf (currentPresetFile)
                                                            : library.getCurrentBank()) + "  /  " + getPresetDisplayName(),
                          engine.getChangeCounter() != savedChangeCounter);

    // A plugin that crashed while processing has been taken out of the signal: say so once.
    for (auto& sl : engine.getFlatSlots())
        if (sl->crashed && reportedCrashes.insert (sl.get()).second)
            setStatus (sl->description.name + " crashed and has been taken out of the signal. Save your work, "
                       "then restart PEQUOD to use it again.");

    // Autosave the session every ~30 s so a crash (usually a misbehaving plugin) loses little.
    if (++autosaveTicks >= 150)
    {
        autosaveTicks = 0;
        saveSettings();
    }
}

void MainComponent::changeListenerCallback (juce::ChangeBroadcaster* source)
{
    if (source == &library)
        control.sendRackSummary();

    if (source == &engine.knownPlugins)
    {
        if (auto plugins = engine.knownPlugins.createXml())
        {
            appProperties->getUserSettings()->setValue ("knownPlugins", plugins.get());
            appProperties->getUserSettings()->saveIfNeeded();
        }
    }
    timerCallback();
}

//==============================================================================
juce::Array<juce::File> MainComponent::getPresetFiles() const
{
    return library.getPresets (library.getCurrentBank());
}

juce::String MainComponent::getPresetDisplayName() const
{
    return currentPresetFile.existsAsFile() ? currentPresetFile.getFileNameWithoutExtension()
                                            : engine.currentPresetName;
}

std::unique_ptr<juce::XmlElement> MainComponent::createFullRackXml() const
{
    auto xml = engine.createRackXml();
    xml->addChildElement (control.createRackMappingsXml().release());
    return xml;
}

void MainComponent::loadRack (const juce::XmlElement& xml)
{
    windows.clear();
    juce::StringArray errors;
    engine.loadRackXml (xml, errors);
    control.loadRackMappingsXml (&xml);

    if (! errors.isEmpty())
        setStatus ("Couldn't load: " + errors.joinIntoString ("; "));
}

void MainComponent::loadPresetFile (const juce::File& f)
{
    auto xml = juce::parseXML (f);
    if (xml == nullptr)
    {
        setStatus ("Couldn't read " + f.getFileName());
        return;
    }

    loadRack (*xml);
    clearUndo();
    currentPresetFile = f;
    library.setCurrentBank (PresetLibrary::bankOf (f));
    savedChangeCounter = engine.getChangeCounter();
    browser.setCurrentPresetFile (f);
    control.sendRackSummary();
    if (statusMessage.startsWith ("Couldn't") == false || juce::Time::getMillisecondCounterHiRes() - statusTime > 200)
        setStatus ("Loaded " + f.getFileNameWithoutExtension());
}

void MainComponent::loadPresetIndex (int index)
{
    auto files = getPresetFiles();
    if (juce::isPositiveAndBelow (index, files.size()))
        loadPresetFile (files[index]);
}

void MainComponent::loadPresetByName (const juce::String& name)
{
    // Current bank first, then every other bank.
    for (auto& f : getPresetFiles())
        if (f.getFileNameWithoutExtension().equalsIgnoreCase (name.trim()))
            return loadPresetFile (f);
    for (auto& bank : library.getBanks())
        for (auto& f : library.getPresets (bank))
            if (f.getFileNameWithoutExtension().equalsIgnoreCase (name.trim()))
                return loadPresetFile (f);
    setStatus ("No preset called " + name);
}

void MainComponent::stepPreset (int delta)
{
    auto files = getPresetFiles();
    if (files.isEmpty()) { setStatus ("Bank " + library.getCurrentBank() + " is empty"); return; }
    auto idx = files.indexOf (currentPresetFile);
    idx = idx < 0 ? (delta > 0 ? 0 : files.size() - 1)
                  : (idx + delta + files.size()) % files.size();
    loadPresetFile (files[idx]);
}

//==============================================================================
void MainComponent::selectBank (int index, bool loadPresetToo)
{
    auto banks = library.getBanks();
    if (! juce::isPositiveAndBelow (index, banks.size())) return;

    // Keep the same slot number when moving between banks (handy with footswitches).
    const int slot = juce::jmax (0, getPresetFiles().indexOf (currentPresetFile));
    library.setCurrentBank (banks[index]);

    if (loadPresetToo)
    {
        auto files = getPresetFiles();
        if (! files.isEmpty())
            loadPresetFile (files[juce::jmin (slot, files.size() - 1)]);
        else
            setStatus ("Bank " + banks[index] + " is empty");
    }
    else
    {
        setStatus ("Bank " + banks[index]);
    }
}

void MainComponent::stepBank (int delta)
{
    auto banks = library.getBanks();
    if (banks.isEmpty()) return;
    auto idx = library.getCurrentBankIndex();
    selectBank ((juce::jmax (0, idx) + delta + banks.size()) % banks.size(), true);
}

void MainComponent::askForText (const juce::String& title, const juce::String& label, const juce::String& initial,
                                std::function<void (const juce::String&)> onOk)
{
    auto* w = new juce::AlertWindow (title, label, juce::MessageBoxIconType::NoIcon, this);
    w->addTextEditor ("text", initial);
    w->addButton ("OK", 1, juce::KeyPress (juce::KeyPress::returnKey));
    w->addButton ("CANCEL", 0, juce::KeyPress (juce::KeyPress::escapeKey));
    w->enterModalState (true, juce::ModalCallbackFunction::create ([w, onOk] (int result)
    {
        if (result == 1) onOk (w->getTextEditorContents ("text").trim());
    }), true);

    if (auto* ed = w->getTextEditor ("text"))
    {
        ed->setFont (Pequod::body (15.0f));
        ed->selectAll();
        ed->grabKeyboardFocus();
    }
}

void MainComponent::newBank()
{
    askForText ("NEW BANK", "Bank name", "Bank " + juce::String (library.getBanks().size() + 1), [this] (const juce::String& name)
    {
        auto err = library.createBank (name);
        setStatus (err.isEmpty() ? "Created bank " + name : err);
    });
}

void MainComponent::renameBank (const juce::String& bank)
{
    askForText ("RENAME BANK", "New name for " + bank, bank, [this, bank] (const juce::String& name)
    {
        const bool currentInside = PresetLibrary::bankOf (currentPresetFile) == bank && currentPresetFile.existsAsFile();
        const auto fileName = currentPresetFile.getFileName();
        auto err = library.renameBank (bank, name);
        if (err.isNotEmpty()) { setStatus (err); return; }
        if (currentInside)
            currentPresetFile = library.getBankFolder (juce::File::createLegalFileName (name.trim())).getChildFile (fileName);
        browser.setCurrentPresetFile (currentPresetFile);
        setStatus ("Renamed bank to " + name);
    });
}

void MainComponent::deleteBank (const juce::String& bank)
{
    const int count = library.getPresets (bank).size();
    auto options = juce::MessageBoxOptions::makeOptionsOkCancel (juce::MessageBoxIconType::WarningIcon, "Delete bank",
                                                                 "Move bank \"" + bank + "\" and its " + juce::String (count)
                                                                     + " preset(s) to the recycle bin?",
                                                                 "DELETE", "CANCEL", this);
    juce::AlertWindow::showAsync (options, [this, bank] (int result)
    {
        if (result != 1) return;
        if (PresetLibrary::bankOf (currentPresetFile) == bank) currentPresetFile = juce::File();
        auto err = library.deleteBank (bank);
        setStatus (err.isEmpty() ? "Deleted bank " + bank : err);
        browser.setCurrentPresetFile (currentPresetFile);
    });
}

void MainComponent::movePreset (const juce::File& f, const juce::String& bank)
{
    juce::File moved;
    auto err = library.movePreset (f, bank, moved);
    if (err.isNotEmpty()) { setStatus (err); return; }
    if (f == currentPresetFile)
    {
        currentPresetFile = moved;
        library.setCurrentBank (bank);
    }
    browser.setCurrentPresetFile (currentPresetFile);
    setStatus ("Moved " + f.getFileNameWithoutExtension() + " to " + bank);
}

void MainComponent::savePreset (const juce::File& f)
{
    engine.currentPresetName = f.getFileNameWithoutExtension();
    auto xml = createFullRackXml();
    if (! xml->writeTo (f))
    {
        setStatus ("Couldn't write " + f.getFullPathName());
        return;
    }
    currentPresetFile = f;
    savedChangeCounter = engine.getChangeCounter();
    library.setCurrentBank (PresetLibrary::bankOf (f));
    library.refresh();
    browser.setCurrentPresetFile (f);
    control.sendRackSummary();
    setStatus ("Saved " + f.getFileNameWithoutExtension());
}

void MainComponent::save()
{
    if (currentPresetFile.existsAsFile()) savePreset (currentPresetFile);
    else saveAs();
}

void MainComponent::saveAs()
{
    auto* w = new juce::AlertWindow ("SAVE PRESET", "Preset name", juce::MessageBoxIconType::NoIcon, this);
    w->addTextEditor ("name", currentPresetFile.existsAsFile() ? currentPresetFile.getFileNameWithoutExtension() : juce::String());
    auto banks = library.getBanks();
    w->addComboBox ("bank", banks, "Bank");
    if (auto* cb = w->getComboBoxComponent ("bank"))
        cb->setSelectedItemIndex (juce::jmax (0, banks.indexOf (library.getCurrentBank())), juce::dontSendNotification);
    w->addButton ("SAVE", 1, juce::KeyPress (juce::KeyPress::returnKey));
    w->addButton ("CANCEL", 0, juce::KeyPress (juce::KeyPress::escapeKey));

    w->enterModalState (true, juce::ModalCallbackFunction::create ([this, w, banks] (int result)
    {
        if (result != 1) return;
        auto name = juce::File::createLegalFileName (w->getTextEditorContents ("name").trim());
        if (name.isEmpty()) return;
        auto bank = library.getCurrentBank();
        if (auto* cb = w->getComboBoxComponent ("bank"))
            if (juce::isPositiveAndBelow (cb->getSelectedItemIndex(), banks.size()))
                bank = banks[cb->getSelectedItemIndex()];
        savePreset (library.getBankFolder (bank).getChildFile (name + PresetLibrary::extension));
    }), true);

    if (auto* ed = w->getTextEditor ("name"))
    {
        ed->setFont (Pequod::body (15.0f));
        ed->selectAll();
        ed->grabKeyboardFocus();
    }
}

void MainComponent::initRack()
{
    windows.clear();
    pushUndo();
    engine.clearRack();
    control.clearRackMappings();
    engine.currentPresetName = "Init";
    currentPresetFile = juce::File();
    savedChangeCounter = engine.getChangeCounter();
    browser.setCurrentPresetFile ({});
    setStatus ("Empty rack");
}

void MainComponent::deletePreset (const juce::File& f)
{
    auto options = juce::MessageBoxOptions::makeOptionsOkCancel (juce::MessageBoxIconType::WarningIcon,
                                                                 "Delete preset",
                                                                 "Move \"" + f.getFileNameWithoutExtension() + "\" to the recycle bin?",
                                                                 "DELETE", "CANCEL", this);
    juce::AlertWindow::showAsync (options, [this, f] (int result)
    {
        if (result != 1) return;
        f.moveToTrash();
        if (f == currentPresetFile) currentPresetFile = juce::File();
        library.refresh();
    });
}

void MainComponent::showPresetMenu()
{
    juce::PopupMenu m;
    for (auto& bank : library.getBanks())
    {
        juce::PopupMenu sub;
        auto files = library.getPresets (bank);
        for (int i = 0; i < files.size(); ++i)
            sub.addItem (juce::String (i + 1).paddedLeft ('0', 2) + "  " + files[i].getFileNameWithoutExtension(),
                         true, files[i] == currentPresetFile, [this, f = files[i]] { loadPresetFile (f); });
        if (files.isEmpty())
            sub.addItem ("(empty)", false, false, nullptr);
        m.addSubMenu (bank, sub, true, nullptr, bank == library.getCurrentBank());
    }
    m.addSeparator();
    m.addItem ("New bank...", [this] { newBank(); });
    m.addItem ("Open presets folder", [this] { library.getRoot().revealToUser(); });
    m.showMenuAsync (juce::PopupMenu::Options().withTargetComponent (this).withMousePosition());
}

//==============================================================================
void MainComponent::addPluginAtEnd (const juce::PluginDescription& d)
{
    juce::String err;
    if (engine.addPlugin (d, { -1, -1, 0 }, err) == 0)
        setStatus ("Couldn't load " + d.name + ": " + err);
    else
        setStatus ("Added " + d.name);
}

void MainComponent::openPluginWindow (int uid)
{
    if (auto it = windows.find (uid); it != windows.end())
    {
        it->second->toFront (true);
        return;
    }

    auto slot = engine.findSlot (uid);
    if (slot == nullptr) return;
    auto* inst = slot->instance();
    if (inst == nullptr) return;

    rack.dropEditor (uid);
    if (inst->getActiveEditor() != nullptr) return;

    juce::AudioProcessorEditor* editor = nullptr;
    {
        CrashGuard::Scope guard (slot->nameUtf8.c_str(), "opening its window");
        editor = inst->hasEditor() ? inst->createEditorAndMakeActive()
                                   : new juce::GenericAudioProcessorEditor (*inst);
    }
    if (editor == nullptr) return;

    windows[uid] = std::make_unique<PluginWindow> (slot->description.name, editor,
                                                   [this, uid] { juce::MessageManager::callAsync ([this, uid] { closePluginWindow (uid); }); });
}

void MainComponent::closePluginWindow (int uid)
{
    windows.erase (uid);
}

void MainComponent::showAudioSettings()
{
    juce::DialogWindow::LaunchOptions o;
    o.content.setOwned (new SettingsPanel (engine, control));
    o.dialogTitle = "Audio, MIDI & OSC settings";
    o.dialogBackgroundColour = Pequod::Colours::panel;
    o.useNativeTitleBar = true;
    o.resizable = true;
    o.componentToCentreAround = this;
    o.launchAsync();
}

void MainComponent::showController()
{
    if (controllerWindow == nullptr)
        controllerWindow = std::make_unique<ControllerWindow> (*controller, engine, control);
    controllerWindow->setVisible (true);
    controllerWindow->toFront (true);
}

void MainComponent::showPluginManager (bool autoScan)
{
    auto deadMansPedal = appProperties->getUserSettings()->getFile().getSiblingFile ("RecentlyCrashedPluginsList");
    auto* panel = new PluginManagerPanel (engine, *appProperties->getUserSettings(), deadMansPedal);
    panel->onStatus = [this] (const juce::String& s) { setStatus (s); };

    juce::DialogWindow::LaunchOptions o;
    o.content.setOwned (panel);
    o.dialogTitle = "Plugins";
    o.dialogBackgroundColour = Pequod::Colours::panel;
    o.useNativeTitleBar = true;
    o.resizable = true;
    o.componentToCentreAround = this;
    o.launchAsync();

    if (autoScan)
    {
        setStatus ("First run: scanning your VST3 folders...");
        panel->scanFolders();
    }
}

//==============================================================================
void MainComponent::pushUndo()
{
    if (restoringUndo) return;
    undoStack.push_back (createFullRackXml());
    constexpr size_t maxUndo = 40;
    if (undoStack.size() > maxUndo) undoStack.erase (undoStack.begin());
    redoStack.clear();
    updateUndoButtons();
}

void MainComponent::clearUndo()
{
    undoStack.clear();
    redoStack.clear();
    updateUndoButtons();
}

void MainComponent::restoreSnapshot (const juce::XmlElement& xml)
{
    restoringUndo = true;
    const auto name = engine.currentPresetName;
    loadRack (xml);
    engine.currentPresetName = name;
    engine.markModified();
    control.sendRackSummary();
    restoringUndo = false;
    updateUndoButtons();
}

void MainComponent::undo()
{
    if (undoStack.empty()) { setStatus ("Nothing to undo"); return; }
    redoStack.push_back (createFullRackXml());
    auto snap = std::move (undoStack.back());
    undoStack.pop_back();
    restoreSnapshot (*snap);
    setStatus ("Undone");
}

void MainComponent::redo()
{
    if (redoStack.empty()) { setStatus ("Nothing to redo"); return; }
    undoStack.push_back (createFullRackXml());
    auto snap = std::move (redoStack.back());
    redoStack.pop_back();
    restoreSnapshot (*snap);
    setStatus ("Redone");
}

void MainComponent::updateUndoButtons()
{
    undoButton.setEnabled (! undoStack.empty());
    redoButton.setEnabled (! redoStack.empty());
}

//==============================================================================
void MainComponent::askAfterCrash (const CrashGuard::LastRun& last)
{
    auto* settings = appProperties->getUserSettings();
    auto session = settings->getXmlValue ("session");

    auto loadSession = [this] (const juce::StringArray& skip)
    {
        auto* st = appProperties->getUserSettings();
        if (auto xml = st->getXmlValue ("session"))
        {
            engine.skipPlugins = skip;
            loadRack (*xml);
            engine.skipPlugins.clear();
            if (skip.isEmpty() && ! st->getBoolValue ("sessionModified", false))
                savedChangeCounter = engine.getChangeCounter();
        }
    };

    if (session == nullptr || session->getNumChildElements() == 0)
    {
        waitingForCrashAnswer = false;           // nothing was loaded: nothing to protect against
        return;
    }

    const bool known = last.plugin.isNotEmpty();
    juce::String msg = "PEQUOD Host didn't close properly last time";
    msg << (known ? " - it crashed inside " + last.plugin.quoted() + (last.doing.isNotEmpty() ? " while " + last.doing : juce::String()) + "."
                  : ".");
    msg << "\n\nHow do you want to start?";

    auto options = juce::MessageBoxOptions()
                       .withIconType (juce::MessageBoxIconType::NoIcon)
                       .withTitle ("PEQUOD Host")
                       .withMessage (msg)
                       .withAssociatedComponent (this);
    options = known ? options.withButton ("Without " + last.plugin).withButton ("Empty rack").withButton ("Load normally")
                    : options.withButton ("Load normally").withButton ("Empty rack");

    juce::AlertWindow::showAsync (options, [this, known, last, loadSession] (int result)
    {
        // JUCE numbers the buttons 1, 2, ... with the last one 0.
        waitingForCrashAnswer = false;
        const bool emptyRack = known ? result == 2 : result == 0;
        const bool without = known && result == 1;
        if (emptyRack)
        {
            initRack();
            setStatus ("Started with an empty rack. Your presets are untouched.");
        }
        else if (without)
        {
            loadSession (juce::StringArray (last.plugin));
            setStatus ("Loaded the last session without " + last.plugin + ". Your saved presets are untouched.");
        }
        else
        {
            loadSession ({});
        }
    });
}

void MainComponent::showKeyboard (bool show)
{
    keyboard->setVisible (show);
    keysButton.setToggleState (show, juce::dontSendNotification);
    if (! show) engine.getRack().keyboardState.allNotesOff (0);
    resized();
    if (show)
    {
        keyboard->setLowestVisibleKey (48);        // from C below middle C
        keyboard->grabKeyboardFocus();
    }
}
