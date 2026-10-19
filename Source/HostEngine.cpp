#include "HostEngine.h"

using Graph  = juce::AudioProcessorGraph;
using NodeID = juce::AudioProcessorGraph::NodeID;

//==============================================================================
struct PluginSlot::Listener : public juce::AudioProcessorParameter::Listener
{
    Listener (int uidIn, ParamObserver* const* obsIn) : uid (uidIn), observer (obsIn) {}

    void parameterValueChanged (int index, float value) override
    {
        if (auto* o = *observer) o->pluginParameterChanged (uid, index, value);
    }

    void parameterGestureChanged (int index, bool starting) override
    {
        if (starting)
            if (auto* o = *observer) o->pluginParameterGesture (uid, index);
    }

    int uid;
    ParamObserver* const* observer;
};

//==============================================================================
HostEngine::HostEngine()
{
    juce::addDefaultFormatsToManager (formatManager);

    audioIn  = graph.addNode (std::make_unique<Graph::AudioGraphIOProcessor> (Graph::AudioGraphIOProcessor::audioInputNode));
    audioOut = graph.addNode (std::make_unique<Graph::AudioGraphIOProcessor> (Graph::AudioGraphIOProcessor::audioOutputNode));
    midiIn   = graph.addNode (std::make_unique<Graph::AudioGraphIOProcessor> (Graph::AudioGraphIOProcessor::midiInputNode));

    auto in = std::make_unique<GainMeterProcessor>();
    in->inputMode = (int) GainMeterProcessor::InputMode::Left; // guitar into input 1 by default
    in->tapEnabled = true;
    inputStage = in.get();
    inNode = graph.addNode (std::move (in));

    auto out = std::make_unique<GainMeterProcessor>();
    outputStage = out.get();
    outNode = graph.addNode (std::move (out));

    rewire();
}

HostEngine::~HostEngine()
{
    deviceManager.removeChangeListener (this);
    deviceManager.removeMidiInputDeviceCallback ({}, &player);
    deviceManager.removeAudioCallback (&player);
    player.setProcessor (nullptr);
    clearRack();
    graph.clear();
}

void HostEngine::initialiseAudio (const juce::XmlElement* savedDeviceState)
{
    auto err = deviceManager.initialise (2, 2, savedDeviceState, true);
    if (err.isNotEmpty())
        DBG ("Audio init: " << err);

    if (savedDeviceState == nullptr)
        for (auto& m : juce::MidiInput::getAvailableDevices())
            deviceManager.setMidiInputDeviceEnabled (m.identifier, true);

    player.setProcessor (&graph);
    deviceManager.addAudioCallback (&player);
    deviceManager.addMidiInputDeviceCallback ({}, &player);
    deviceManager.addChangeListener (this);
    rewire();
}

void HostEngine::changeListenerCallback (juce::ChangeBroadcaster*)
{
    // Device / channel layout may have changed.
    rewire();
}

//==============================================================================
int HostEngine::numOuts (const Graph::Node::Ptr& n) const
{
    if (n == nullptr) return 0;
    auto* p = n->getProcessor();
    if (n == audioIn) return graph.getTotalNumInputChannels();
    return p->getMainBusNumOutputChannels();
}

int HostEngine::numIns (const Graph::Node::Ptr& n) const
{
    if (n == nullptr) return 0;
    auto* p = n->getProcessor();
    if (n == audioOut) return graph.getTotalNumOutputChannels();
    return p->getMainBusNumInputChannels();
}

void HostEngine::connectAudio (NodeID src, int srcChans, NodeID dst, int dstChans, int dstOffset)
{
    if (srcChans <= 0 || dstChans <= 0) return;

    auto add = [&] (int s, int d) { graph.addConnection ({ { src, s }, { dst, d + dstOffset } }, Graph::UpdateKind::none); };

    if (srcChans >= 2 && dstChans >= 2)      { add (0, 0); add (1, 1); }
    else if (srcChans == 1 && dstChans >= 2) { add (0, 0); add (0, 1); }
    else if (srcChans >= 2 && dstChans == 1) { add (0, 0); add (1, 0); }
    else                                     { add (0, 0); }
}

void HostEngine::rewire()
{
    for (auto& c : graph.getConnections())
        graph.removeConnection (c, Graph::UpdateKind::none);

    connectAudio (audioIn->nodeID, numOuts (audioIn), inNode->nodeID, 2);

    auto chainPlugin = [this] (Graph::Node::Ptr prev, const std::shared_ptr<PluginSlot>& slot) -> Graph::Node::Ptr
    {
        auto node = slot->node;
        if (node == nullptr) return prev;

        if (node->getProcessor()->acceptsMidi())
            graph.addConnection ({ { midiIn->nodeID, Graph::midiChannelIndex }, { node->nodeID, Graph::midiChannelIndex } },
                                 Graph::UpdateKind::none);

        if (numOuts (node) == 0)      // MIDI-only plugin: audio passes around it
            return prev;

        connectAudio (prev->nodeID, numOuts (prev), node->nodeID, numIns (node));
        return node;
    };

    Graph::Node::Ptr prev = inNode;

    for (auto& item : items)
    {
        if (item.type == RackItem::Type::plugin)
        {
            prev = chainPlugin (prev, item.plugin);
        }
        else if (item.mixerNode != nullptr)
        {
            for (int lane = 0; lane < 2; ++lane)
            {
                auto lanePrev = prev;
                for (auto& s : item.lanes[lane])
                    lanePrev = chainPlugin (lanePrev, s);

                connectAudio (lanePrev->nodeID, numOuts (lanePrev), item.mixerNode->nodeID, 2, lane * 2);
            }
            prev = item.mixerNode;
        }
    }

    connectAudio (prev->nodeID, numOuts (prev), outNode->nodeID, 2);
    connectAudio (outNode->nodeID, 2, audioOut->nodeID, numIns (audioOut));

    graph.rebuild();
}

//==============================================================================
void HostEngine::attachListener (PluginSlot& slot)
{
    if (auto* inst = slot.instance())
    {
        slot.listener = std::make_unique<PluginSlot::Listener> (slot.uid, &observer);
        for (auto* p : inst->getParameters())
            p->addListener (slot.listener.get());
    }
}

std::shared_ptr<PluginSlot> HostEngine::createSlot (const juce::PluginDescription& desc, juce::String& error)
{
    double sr = 48000.0;
    int bs = 512;
    if (auto* dev = deviceManager.getCurrentAudioDevice())
    {
        sr = dev->getCurrentSampleRate();
        bs = dev->getCurrentBufferSizeSamples();
    }

    auto inst = formatManager.createPluginInstance (desc, sr, bs, error);
    if (inst == nullptr)
    {
        if (error.isEmpty()) error = "Could not load " + desc.name;
        return {};
    }

    // Prefer stereo main buses; plugins that refuse keep their own layout (mono is handled when wiring).
    for (bool isInput : { true, false })
        if (auto* bus = inst->getBus (isInput, 0))
            if (bus->getNumberOfChannels() != 2)
                bus->setCurrentLayout (juce::AudioChannelSet::stereo());

    auto slot = std::make_shared<PluginSlot>();
    slot->uid = nextUid++;
    slot->description = desc;
    slot->node = graph.addNode (std::move (inst), std::nullopt, Graph::UpdateKind::none);
    attachListener (*slot);
    return slot;
}

void HostEngine::destroySlot (const std::shared_ptr<PluginSlot>& slot)
{
    if (slot == nullptr) return;

    if (onSlotAboutToBeRemoved)
        onSlotAboutToBeRemoved (slot->uid);

    if (auto* inst = slot->instance())
        if (slot->listener != nullptr)
            for (auto* p : inst->getParameters())
                p->removeListener (slot->listener.get());

    if (slot->node != nullptr)
        graph.removeNode (slot->node->nodeID); // synchronous: audio thread stops using it first

    slot->node = nullptr;
    slot->listener.reset();
}

//==============================================================================
int HostEngine::addPlugin (const juce::PluginDescription& desc, RackLocation where, juce::String& errorOut)
{
    auto slot = createSlot (desc, errorOut);
    if (slot == nullptr) return 0;

    insertSlot (slot, where);
    rewire();
    changed();
    return slot->uid;
}

void HostEngine::insertSlot (std::shared_ptr<PluginSlot> slot, RackLocation where)
{
    if (where.lane >= 0 && juce::isPositiveAndBelow (where.itemIndex, (int) items.size())
         && items[(size_t) where.itemIndex].type == RackItem::Type::split)
    {
        auto& lane = items[(size_t) where.itemIndex].lanes[juce::jlimit (0, 1, where.lane)];
        auto pos = juce::jlimit (0, (int) lane.size(), where.laneIndex);
        lane.insert (lane.begin() + pos, std::move (slot));
        return;
    }

    RackItem item;
    item.uid = nextUid++;
    item.type = RackItem::Type::plugin;
    item.plugin = std::move (slot);
    auto pos = juce::jlimit (0, (int) items.size(), where.itemIndex < 0 ? (int) items.size() : where.itemIndex);
    items.insert (items.begin() + pos, std::move (item));
}

int HostEngine::addSplit (int itemIndex)
{
    RackItem item;
    item.uid = nextUid++;
    item.type = RackItem::Type::split;
    auto mixer = std::make_unique<SplitMixerProcessor>();
    item.mixerNode = graph.addNode (std::move (mixer), std::nullopt, Graph::UpdateKind::none);

    auto pos = juce::jlimit (0, (int) items.size(), itemIndex < 0 ? (int) items.size() : itemIndex);
    auto uid = item.uid;
    items.insert (items.begin() + pos, std::move (item));
    rewire();
    changed();
    return uid;
}

std::shared_ptr<PluginSlot> HostEngine::detachSlot (int slotUid)
{
    for (auto it = items.begin(); it != items.end(); ++it)
    {
        if (it->type == RackItem::Type::plugin && it->plugin && it->plugin->uid == slotUid)
        {
            auto s = it->plugin;
            items.erase (it);
            return s;
        }

        if (it->type == RackItem::Type::split)
            for (auto& lane : it->lanes)
                for (auto l = lane.begin(); l != lane.end(); ++l)
                    if ((*l)->uid == slotUid)
                    {
                        auto s = *l;
                        lane.erase (l);
                        return s;
                    }
    }
    return {};
}

void HostEngine::removeSlot (int slotUid)
{
    if (auto s = detachSlot (slotUid))
    {
        destroySlot (s);
        rewire();
        changed();
    }
}

void HostEngine::removeItem (int itemUid)
{
    for (auto it = items.begin(); it != items.end(); ++it)
    {
        if (it->uid != itemUid) continue;

        if (it->type == RackItem::Type::plugin)
            destroySlot (it->plugin);
        else
        {
            for (auto& lane : it->lanes)
                for (auto& s : lane)
                    destroySlot (s);
            if (it->mixerNode != nullptr)
                graph.removeNode (it->mixerNode->nodeID);
        }

        items.erase (it);
        rewire();
        changed();
        return;
    }
}

void HostEngine::moveSlot (int slotUid, RackLocation where)
{
    RackLocation from;
    if (! locateSlot (slotUid, from)) return;

    // Indices were computed against the rack before removal, so compensate.
    if (from.lane < 0 && where.lane < 0 && from.itemIndex < where.itemIndex)
        --where.itemIndex;
    if (from.lane >= 0 && where.lane == from.lane && where.itemIndex == from.itemIndex && from.laneIndex < where.laneIndex)
        --where.laneIndex;
    if (from.lane < 0 && where.lane >= 0 && from.itemIndex < where.itemIndex)
        --where.itemIndex; // the top-level item above the split disappears

    auto s = detachSlot (slotUid);
    if (s == nullptr) return;

    insertSlot (s, where);
    rewire();
    changed();
}

void HostEngine::moveItem (int itemUid, int newIndex)
{
    for (int i = 0; i < (int) items.size(); ++i)
    {
        if (items[(size_t) i].uid != itemUid) continue;
        if (i < newIndex) --newIndex;
        auto item = std::move (items[(size_t) i]);
        items.erase (items.begin() + i);
        newIndex = juce::jlimit (0, (int) items.size(), newIndex);
        items.insert (items.begin() + newIndex, std::move (item));
        rewire();
        changed();
        return;
    }
}

void HostEngine::clearRack()
{
    for (auto& item : items)
    {
        if (item.type == RackItem::Type::plugin)
            destroySlot (item.plugin);
        else
        {
            for (auto& lane : item.lanes)
                for (auto& s : lane)
                    destroySlot (s);
            if (item.mixerNode != nullptr)
                graph.removeNode (item.mixerNode->nodeID);
        }
    }
    items.clear();
    rewire();
    changed();
}

void HostEngine::setBypassed (int slotUid, bool shouldBypass)
{
    if (auto s = findSlot (slotUid))
    {
        s->bypassed = shouldBypass;
        if (s->node != nullptr)
            s->node->setBypassed (shouldBypass);
        changed();
    }
}

void HostEngine::setSplitMix (int itemUid, float mix)
{
    if (auto* item = findItem (itemUid))
    {
        item->mix = juce::jlimit (0.0f, 1.0f, mix);
        if (auto* m = item->mixer())
            m->mix = item->mix;
    }
}

//==============================================================================
std::shared_ptr<PluginSlot> HostEngine::findSlot (int slotUid) const
{
    for (auto& item : items)
    {
        if (item.type == RackItem::Type::plugin && item.plugin && item.plugin->uid == slotUid)
            return item.plugin;
        for (auto& lane : item.lanes)
            for (auto& s : lane)
                if (s->uid == slotUid)
                    return s;
    }
    return {};
}

RackItem* HostEngine::findItem (int itemUid)
{
    for (auto& item : items)
        if (item.uid == itemUid)
            return &item;
    return nullptr;
}

bool HostEngine::locateSlot (int slotUid, RackLocation& out) const
{
    for (int i = 0; i < (int) items.size(); ++i)
    {
        auto& item = items[(size_t) i];
        if (item.type == RackItem::Type::plugin && item.plugin && item.plugin->uid == slotUid)
        {
            out = { i, -1, 0 };
            return true;
        }
        for (int l = 0; l < 2; ++l)
            for (int j = 0; j < (int) item.lanes[l].size(); ++j)
                if (item.lanes[l][(size_t) j]->uid == slotUid)
                {
                    out = { i, l, j };
                    return true;
                }
    }
    return false;
}

std::vector<std::shared_ptr<PluginSlot>> HostEngine::getFlatSlots() const
{
    std::vector<std::shared_ptr<PluginSlot>> out;
    for (auto& item : items)
    {
        if (item.type == RackItem::Type::plugin) out.push_back (item.plugin);
        else for (auto& lane : item.lanes) for (auto& s : lane) out.push_back (s);
    }
    return out;
}

int HostEngine::getFlatIndex (int slotUid) const
{
    int i = 1;
    for (auto& s : getFlatSlots())
    {
        if (s->uid == slotUid) return i;
        ++i;
    }
    return 0;
}

std::vector<RackItem*> HostEngine::getSplits()
{
    std::vector<RackItem*> out;
    for (auto& item : items)
        if (item.type == RackItem::Type::split)
            out.push_back (&item);
    return out;
}

int HostEngine::getSplitNumber (int itemUid) const
{
    int n = 0;
    for (auto& item : items)
        if (item.type == RackItem::Type::split)
        {
            ++n;
            if (item.uid == itemUid) return n;
        }
    return 0;
}

//==============================================================================
static std::unique_ptr<juce::XmlElement> slotToXml (const PluginSlot& s)
{
    auto x = std::make_unique<juce::XmlElement> ("SLOT");
    x->setAttribute ("bypass", s.bypassed);
    x->addChildElement (s.description.createXml().release());

    if (auto* inst = s.instance())
    {
        juce::MemoryBlock mb;
        inst->getStateInformation (mb);
        x->createNewChildElement ("STATE")->addTextElement (mb.toBase64Encoding());
    }
    return x;
}

std::unique_ptr<juce::XmlElement> HostEngine::createRackXml() const
{
    auto root = std::make_unique<juce::XmlElement> ("PEQUODRACK");
    root->setAttribute ("version", 1);
    root->setAttribute ("name", currentPresetName);
    root->setAttribute ("inGain", inputStage->gainDb.load());
    root->setAttribute ("outGain", outputStage->gainDb.load());
    root->setAttribute ("inputMode", inputStage->inputMode.load());

    for (auto& item : items)
    {
        auto* xi = root->createNewChildElement ("ITEM");
        if (item.type == RackItem::Type::plugin)
        {
            xi->setAttribute ("type", "plugin");
            xi->addChildElement (slotToXml (*item.plugin).release());
        }
        else
        {
            xi->setAttribute ("type", "split");
            xi->setAttribute ("mix", item.mix);
            for (int l = 0; l < 2; ++l)
            {
                auto* xl = xi->createNewChildElement ("LANE");
                xl->setAttribute ("id", l);
                for (auto& s : item.lanes[l])
                    xl->addChildElement (slotToXml (*s).release());
            }
        }
    }
    return root;
}

bool HostEngine::loadRackXml (const juce::XmlElement& root, juce::StringArray& errors)
{
    if (! root.hasTagName ("PEQUODRACK"))
    {
        errors.add ("Not a PEQUOD rack preset");
        return false;
    }

    // Tear down without rewiring/broadcasting in between.
    for (auto& item : items)
    {
        if (item.type == RackItem::Type::plugin) destroySlot (item.plugin);
        else
        {
            for (auto& lane : item.lanes) for (auto& s : lane) destroySlot (s);
            if (item.mixerNode != nullptr) graph.removeNode (item.mixerNode->nodeID);
        }
    }
    items.clear();

    currentPresetName = root.getStringAttribute ("name", "Untitled");
    inputStage->gainDb  = (float) root.getDoubleAttribute ("inGain", 0.0);
    outputStage->gainDb = (float) root.getDoubleAttribute ("outGain", 0.0);
    inputStage->inputMode = root.getIntAttribute ("inputMode", (int) GainMeterProcessor::InputMode::Left);

    auto makeSlot = [&] (const juce::XmlElement& xs) -> std::shared_ptr<PluginSlot>
    {
        juce::PluginDescription desc;
        if (auto* xd = xs.getChildByName ("PLUGIN"))
            desc.loadFromXml (*xd);

        // Prefer the locally-scanned description (path may differ between machines).
        for (auto& known : knownPlugins.getTypes())
            if (known.isDuplicateOf (desc)) { desc = known; break; }

        juce::String err;
        auto slot = createSlot (desc, err);
        if (slot == nullptr)
        {
            errors.add (desc.name + ": " + err);
            return {};
        }

        if (auto* xst = xs.getChildByName ("STATE"))
        {
            juce::MemoryBlock mb;
            if (mb.fromBase64Encoding (xst->getAllSubText().trim()))
                slot->instance()->setStateInformation (mb.getData(), (int) mb.getSize());
        }

        slot->bypassed = xs.getBoolAttribute ("bypass", false);
        slot->node->setBypassed (slot->bypassed);
        return slot;
    };

    for (auto* xi : root.getChildWithTagNameIterator ("ITEM"))
    {
        RackItem item;
        item.uid = nextUid++;

        if (xi->getStringAttribute ("type") == "split")
        {
            item.type = RackItem::Type::split;
            item.mix = (float) xi->getDoubleAttribute ("mix", 0.5);
            auto mixer = std::make_unique<SplitMixerProcessor>();
            mixer->mix = item.mix;
            item.mixerNode = graph.addNode (std::move (mixer), std::nullopt, Graph::UpdateKind::none);

            for (auto* xl : xi->getChildWithTagNameIterator ("LANE"))
            {
                auto l = juce::jlimit (0, 1, xl->getIntAttribute ("id"));
                for (auto* xs : xl->getChildWithTagNameIterator ("SLOT"))
                    if (auto s = makeSlot (*xs))
                        item.lanes[l].push_back (s);
            }
            items.push_back (std::move (item));
        }
        else if (auto* xs = xi->getChildByName ("SLOT"))
        {
            if (auto s = makeSlot (*xs))
            {
                item.type = RackItem::Type::plugin;
                item.plugin = s;
                items.push_back (std::move (item));
            }
        }
    }

    rewire();
    changed();
    return errors.isEmpty();
}
