#include "PluginManagerPanel.h"

PluginManagerPanel::PluginManagerPanel (HostEngine& e, juce::PropertiesFile& p, juce::File deadMansPedal)
    : engine (e), props (p)
{
    for (auto* f : engine.formatManager.getFormats())
        if (f->getName() == "VST3")
            vst3 = f;

    auto caption = [] (juce::Label& l, const juce::String& t)
    {
        l.setText (t, juce::dontSendNotification);
        l.setFont (Pequod::heading (12.0f));
        l.setColour (juce::Label::textColourId, Pequod::Colours::mustard);
    };
    caption (foldersCaption, "VST3 FOLDERS");
    caption (listCaption, "INSTALLED PLUGINS");

    foldersHelp.setText ("Folders searched for plugins. Use + to add one, - to remove, or drag a folder in from Explorer.",
                         juce::dontSendNotification);
    foldersHelp.setFont (Pequod::body (12.0f));
    foldersHelp.setColour (juce::Label::textColourId, Pequod::Colours::textDim);

    folders.setColour (juce::FileSearchPathListComponent::backgroundColourId, Pequod::Colours::well);
    if (vst3 != nullptr)
        folders.setPath (juce::PluginListComponent::getLastSearchPath (props, *vst3));

    scanButton.setTooltip ("Look for new or updated plugins in these folders");
    scanButton.onClick = [this] { scanFolders(); };
    defaultsButton.setTooltip ("Put back the standard VST3 folders for this computer");
    defaultsButton.onClick = [this]
    {
        if (vst3 != nullptr)
        {
            folders.setPath (vst3->getDefaultLocationsToSearch());
            savePath();
        }
    };

    pluginList = std::make_unique<juce::PluginListComponent> (engine.formatManager, engine.knownPlugins,
                                                               deadMansPedal, &props, true);

    for (auto* c : std::initializer_list<juce::Component*> { &foldersCaption, &foldersHelp, &folders, &scanButton,
                                                            &defaultsButton, &listCaption, pluginList.get() })
        addAndMakeVisible (c);

    setSize (780, 680);
}

PluginManagerPanel::~PluginManagerPanel()
{
    savePath();
}

void PluginManagerPanel::savePath()
{
    if (vst3 == nullptr) return;
    juce::PluginListComponent::setLastSearchPath (props, *vst3, folders.getPath());
    props.saveIfNeeded();
}

void PluginManagerPanel::scanFolders()
{
    if (vst3 == nullptr) return;
    savePath();

    auto files = vst3->searchPathsForPlugins (folders.getPath(), true, false);
    if (files.isEmpty())
    {
        if (onStatus) onStatus ("No VST3 plugins found in those folders");
        return;
    }

    // Passing the files directly skips JUCE's own "select folders" prompt.
    pluginList->scanFor (*vst3, files);
}

void PluginManagerPanel::paint (juce::Graphics& g)
{
    g.fillAll (Pequod::Colours::panel);
}

void PluginManagerPanel::resized()
{
    auto r = getLocalBounds().reduced (16, 12);
    foldersCaption.setBounds (r.removeFromTop (22));
    foldersHelp.setBounds (r.removeFromTop (20));
    r.removeFromTop (4);
    folders.setBounds (r.removeFromTop (150));
    r.removeFromTop (8);
    auto buttons = r.removeFromTop (28);
    scanButton.setBounds (buttons.removeFromLeft (150));
    buttons.removeFromLeft (8);
    defaultsButton.setBounds (buttons.removeFromLeft (170));
    r.removeFromTop (16);
    listCaption.setBounds (r.removeFromTop (22));
    pluginList->setBounds (r);
}
