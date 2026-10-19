#include "MainComponent.h"
#include "SettingsPanel.h"
#include "PluginManagerPanel.h"

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

    // Plugins
    if (auto xml = settings->getXmlValue ("knownPlugins"))
        engine.knownPlugins.recreateFromXml (*xml);
    engine.knownPlugins.addChangeListener (this);

    // Audio + MIDI
    auto deviceXml = settings->getXmlValue ("audioDevice");
    engine.initialiseAudio (deviceXml.get());

    engine.setPipelineStages (settings->getIntValue ("pipelineStages", 1));

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
    library.addChangeListener (this);

    browser.refresh();

    controlPanel.onShowInRack = [this] (int uid) { rack.showModule (uid); };

    statusBar.setFont (Pequod::heading (12.0f, false));
    statusBar.setColour (juce::Label::textColourId, Pequod::Colours::textDim);

    for (auto* c : std::initializer_list<juce::Component*> { &header, &browser, &rack, &controlPanel, &statusBar })
        addAndMakeVisible (c);

    // Restore last session
    currentPresetFile = juce::File (settings->getValue ("lastPreset"));
    if (! currentPresetFile.existsAsFile())   // saved before banks: it now lives in "Default"
    {
        auto moved = library.getBankFolder ("Default").getChildFile (currentPresetFile.getFileName());
        currentPresetFile = moved.existsAsFile() ? moved : juce::File();
    }
    library.setCurrentBank (currentPresetFile.existsAsFile() ? PresetLibrary::bankOf (currentPresetFile)
                                                             : settings->getValue ("currentBank"));
    if (auto session = settings->getXmlValue ("session"))
        loadRack (*session);
    if (! settings->getBoolValue ("sessionModified", false))
        savedChangeCounter = engine.getChangeCounter();
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
    saveSettings();

    windows.clear();
    rack.collapseAll();
    engine.onSlotAboutToBeRemoved = [this] (int uid) { rack.dropEditor (uid); };
    engine.clearRack();
    engine.onSlotAboutToBeRemoved = nullptr;

    library.removeChangeListener (this);
    control.setTuner (nullptr);
    control.removeChangeListener (this);
    engine.knownPlugins.removeChangeListener (this);
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
    if (auto rackXml = createFullRackXml())
        s->setValue ("session", rackXml.get());

    s->setValue ("lastPreset", currentPresetFile.getFullPathName());
    s->setValue ("currentBank", library.getCurrentBank());
    s->setValue ("pipelineStages", engine.getPipelineStages());
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
    statusBar.setBounds (r.removeFromBottom (24).reduced (10, 0));
    browser.setBounds (r.removeFromLeft (260));
    controlPanel.setBounds (r.removeFromRight (280));
    rack.setBounds (r);
}

bool MainComponent::keyPressed (const juce::KeyPress& k)
{
    if (k == juce::KeyPress ('s', juce::ModifierKeys::commandModifier, 0)) { save(); return true; }
    if (k == juce::KeyPress ('s', juce::ModifierKeys::commandModifier | juce::ModifierKeys::shiftModifier, 0)) { saveAs(); return true; }
    if (k == juce::KeyPress::escapeKey && control.getLearnState() != ControlSurface::LearnState::off) { control.cancelLearn(); return true; }
    return false;
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

    juce::AudioProcessorEditor* editor = inst->hasEditor() ? inst->createEditorAndMakeActive()
                                                           : new juce::GenericAudioProcessorEditor (*inst);
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
