#include "HostEngine.h"
#include "CrashGuard.h"
#include "PequodNative.h"
#include "Modifiers.h"

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
    formatManager.addFormat (new PequodNative::Format());   // PEQUOD's own plugins, built in

    inputStageOwned = std::make_unique<GainMeterProcessor>();
    inputStageOwned->inputMode = (int) GainMeterProcessor::InputMode::Left; // guitar into input 1 by default
    inputStageOwned->tapEnabled = true;
    inputStage = inputStageOwned.get();

    outputStageOwned = std::make_unique<GainMeterProcessor>();
    outputStageOwned->isOutput = true;
    outputStage = outputStageOwned.get();

    rack = std::make_unique<RackProcessor> (*inputStage, *outputStage);
    rack->transport = &transport;
    rack->netOut = &networkOut;
    rack->onNeedsNewPlan = [this] { rewire(); };

    rewire();
    startTimerHz (2);
}

HostEngine::~HostEngine()
{
    stopTimer();
    deviceManager.removeChangeListener (this);
    deviceManager.removeMidiInputDeviceCallback ({}, &player);
    deviceManager.removeAudioCallback (&player);
    player.setProcessor (nullptr);
    rack->onNeedsNewPlan = nullptr;
    rack->setNetworkOutput (false, false);
    networkOut.stop();
    clearRack();
    for (auto& sp : spares) destroySlot (sp, false);
    spares.clear();
    rack->swapPlan (nullptr);
}

void HostEngine::initialiseAudio (const juce::XmlElement* savedDeviceState)
{
    auto err = deviceManager.initialise (2, 2, savedDeviceState, true);
    if (err.isNotEmpty())
        DBG ("Audio init: " << err);

    if (savedDeviceState == nullptr)
        for (auto& m : juce::MidiInput::getAvailableDevices())
            deviceManager.setMidiInputDeviceEnabled (m.identifier, true);

    player.setProcessor (rack.get());
    deviceManager.addAudioCallback (&player);
    deviceManager.addMidiInputDeviceCallback ({}, &player);
    deviceManager.addChangeListener (this);
    rewire();
}

void HostEngine::changeListenerCallback (juce::ChangeBroadcaster*)
{
    if (auto* dev = deviceManager.getCurrentAudioDevice())
        rack->netInputLatency = dev->getInputLatencyInSamples();
    rewire();
}

//==============================================================================
std::unique_ptr<RenderPlugin> HostEngine::makeRenderPlugin (PluginSlot& slot, int maxBlock) const
{
    auto rp = std::make_unique<RenderPlugin>();
    rp->slot = &slot;
    rp->inst = slot.instance();
    rp->numIn = rp->inst->getMainBusNumInputChannels();
    rp->numOut = rp->inst->getMainBusNumOutputChannels();
    rp->numScratch = juce::jlimit (2, 32, juce::jmax (rp->inst->getTotalNumInputChannels(), rp->inst->getTotalNumOutputChannels()));
    rp->wantsMidi = rp->inst->acceptsMidi();
    rp->scratch.setSize (rp->numScratch, maxBlock);
    rp->midi.ensureSize (4096);
    return rp;
}

bool HostEngine::setNetworkOutput (bool on, const juce::String& name, bool sendDry, juce::String& error)
{
    if (auto* dev = deviceManager.getCurrentAudioDevice())
        rack->netInputLatency = dev->getInputLatencyInSamples();
    networkDry = sendDry;
    if (on)
    {
        if (! networkOut.start (name, error)) { rack->setNetworkOutput (false, sendDry); networkOn = false; return false; }
        networkOut.setName (name);
    }
    rack->setNetworkOutput (on, sendDry);
    if (! on) networkOut.stop();
    networkOn = on;
    return true;
}

bool HostEngine::isInstrument (const PluginSlot& slot)
{
    if (slot.description.isInstrument) return true;
    // Some instruments don't say so: no audio input, takes MIDI, makes sound.
    if (auto* inst = slot.instance())
        return inst->getMainBusNumInputChannels() == 0 && inst->acceptsMidi() && inst->getMainBusNumOutputChannels() > 0;
    return false;
}

float HostEngine::getItemLoad (const RackItem& item) const
{
    if (item.type == RackItem::Type::plugin)
        return item.plugin ? item.plugin->cpuLoad.load() : 0.0f;

    float lane[2] = {};
    for (int l = 0; l < 2; ++l)
        for (auto& s : item.lanes[l])
            lane[l] += s->cpuLoad.load();

    // Lanes run side by side when there are worker threads.
    return rack->getNumWorkerThreads() > 0 ? juce::jmax (lane[0], lane[1]) : lane[0] + lane[1];
}

int HostEngine::getMaxPipelineStages() const noexcept
{
    return juce::jlimit (1, 4, rack->getNumWorkerThreads() + 1);
}

void HostEngine::setPipelineStages (int stages)
{
    pipelineStages = juce::jlimit (1, getMaxPipelineStages(), stages);
    rewire();
}

int HostEngine::getPipelineLatencySamples() const noexcept
{
    return (pipelineStages - 1) * rack->getCurrentBlockSize();
}

int HostEngine::computeChainLatency() const
{
    int total = 0;
    for (auto& item : items)
    {
        if (item.type == RackItem::Type::plugin)
        {
            if (item.plugin && item.plugin->instance()) total += item.plugin->instance()->getLatencySamples();
        }
        else
        {
            int lane[2] = {};
            for (int l = 0; l < 2; ++l)
                for (auto& s : item.lanes[l])
                    if (s->instance()) lane[l] += s->instance()->getLatencySamples();
            total += juce::jmax (lane[0], lane[1]);
        }
    }
    return total;
}

void HostEngine::rewire()
{
    const int maxBlock = juce::jmax (32, rack->getCurrentBlockSize());
    auto plan = std::make_unique<RenderPlan>();
    plan->maxBlock = maxBlock;
    plan->sampleRate = rack->getCurrentSampleRate();

    for (auto& item : items)
    {
        auto ri = std::make_unique<RenderItem>();
        ri->owner = rack.get();

        if (item.type == RackItem::Type::plugin)
        {
            if (! item.plugin || ! item.plugin->instance()) continue;
            ri->plugin = makeRenderPlugin (*item.plugin, maxBlock);
        }
        else
        {
            ri->isSplit = true;
            ri->split = item.state.get();
            ri->mix.reset (plan->sampleRate > 0 ? plan->sampleRate : 48000.0, 0.02);
            ri->mix.setCurrentAndTargetValue (item.state->mix.load());

            for (int l = 0; l < 2; ++l)
            {
                auto& lane = ri->lanes[l];
                lane.buffer.setSize (2, maxBlock);
                lane.midi.ensureSize (4096);
                for (auto& s : item.lanes[l])
                    if (s->instance())
                    {
                        lane.plugins.push_back (makeRenderPlugin (*s, maxBlock));
                        lane.latency += s->instance()->getLatencySamples();
                    }
            }

            const int longest = juce::jmax (ri->lanes[0].latency, ri->lanes[1].latency);
            for (auto& lane : ri->lanes)
                lane.align.prepare (longest - lane.latency, maxBlock);
        }

        plan->items.push_back (std::move (ri));
    }

    // Spread the top-level chain over N stages, balancing the measured CPU of each item.
    const int numStages = juce::jlimit (1, juce::jmax (1, (int) plan->items.size()), pipelineStages);
    plan->stages.resize ((size_t) numStages);

    if (numStages == 1)
    {
        currentPartition.clear();
        for (auto& ri : plan->items)
            plan->stages[0].items.push_back (ri.get());
    }
    else
    {
        currentPartition = computePartition (numStages);
        for (size_t i = 0; i < plan->items.size() && i < currentPartition.size(); ++i)
            plan->stages[(size_t) currentPartition[i]].items.push_back (plan->items[i].get());

        plan->stageAudio.resize ((size_t) numStages);
        plan->stageMidi.resize ((size_t) numStages);
        plan->stageCount.assign ((size_t) numStages, 0);
        plan->ring.resize ((size_t) numStages);
        for (int s = 0; s < numStages; ++s)
        {
            plan->stageAudio[(size_t) s].setSize (2, maxBlock);
            plan->stageAudio[(size_t) s].clear();
            plan->stageMidi[(size_t) s].ensureSize (4096);
            plan->ring[(size_t) s] = s;
        }
    }

    for (auto& sl : getFlatSlots())
        if (isInstrument (*sl))
            plan->instruments.push_back (sl.get());

    // Modifiers and the knobs they move (plugins found by their preset id)
    {
        auto slots = getFlatSlots();
        for (auto& m : modifiers)
        {
            auto* mod = &rack->modBank[m.bank];
            plan->modifiers.push_back (mod);
            mod->rateMod = -1.0f; mod->amountMod = -1.0f; mod->macroMod = -1.0f;   // set again by whatever still moves it
            for (auto& a : m.assignments)
            {
                if (a->targetsModifier())
                {
                    const auto id = a->slotId.fromFirstOccurrenceOf (Modifiers::Assignment::modPrefix, false, false);
                    for (auto& other : modifiers)
                        if (other.id == id && other.bank != m.bank)
                        {
                            RenderPlan::ModTarget t;
                            t.targetMod = &rack->modBank[other.bank];
                            t.modParam = a->paramIndex;
                            t.mod = mod;
                            t.assignment = a;
                            plan->modTargets.push_back (std::move (t));
                        }
                    continue;
                }
                for (auto& sl : slots)
                    if (sl->stableId == a->slotId && sl->instance() != nullptr)
                    {
                        auto& params = sl->instance()->getParameters();
                        if (juce::isPositiveAndBelow (a->paramIndex, params.size()))
                        {
                            RenderPlan::ModTarget t;
                            t.param = params[a->paramIndex];
                            t.mod = mod;
                            t.assignment = a;
                            plan->modTargets.push_back (std::move (t));
                        }
                        break;
                    }
            }
        }
    }

    plan->pluginLatency = computeChainLatency();
    lastPlanLatency = plan->pluginLatency;

    scheduleRebalance();

    auto old = rack->swapPlan (std::move (plan));
    old.reset(); // freed here, on the message thread, after the audio thread has let go of it
}

std::vector<int> HostEngine::computePartition (int numStages) const
{
    std::vector<float> cost;
    for (auto& item : items)
    {
        if (item.type == RackItem::Type::plugin && (! item.plugin || ! item.plugin->instance())) continue;
        cost.push_back (juce::jmax (0.001f, getItemLoad (item)));   // unmeasured items count as "small"
    }

    std::vector<int> stageOf (cost.size(), 0);
    if (cost.empty() || numStages <= 1) return stageOf;

    // Minimise the busiest stage: binary-search the smallest per-stage budget that fits
    // the chain into numStages contiguous groups.
    float lo = 0, hi = 0;
    for (auto c : cost) { lo = juce::jmax (lo, c); hi += c; }

    auto fits = [&] (float budget)
    {
        int stages = 1; float acc = 0;
        for (auto c : cost)
        {
            if (acc + c > budget) { ++stages; acc = 0; }
            acc += c;
        }
        return stages <= numStages;
    };

    for (int iter = 0; iter < 40; ++iter)
    {
        const float mid = 0.5f * (lo + hi);
        if (fits (mid)) hi = mid; else lo = mid;
    }

    int stage = 0; float acc = 0;
    for (size_t i = 0; i < cost.size(); ++i)
    {
        const int itemsLeft = (int) (cost.size() - i);
        const int stagesLeft = numStages - stage - 1;
        // Move on when over budget, or when every later stage still needs an item.
        if (i > 0 && stage < numStages - 1 && (acc + cost[i] > hi * 1.0001f || itemsLeft <= stagesLeft))
        {
            ++stage;
            acc = 0;
        }
        stageOf[i] = stage;
        acc += cost[i];
    }
    return stageOf;
}

float HostEngine::busiestStageLoad (const std::vector<int>& partition) const
{
    std::vector<float> stageLoad (8, 0.0f);
    size_t i = 0;
    for (auto& item : items)
    {
        if (item.type == RackItem::Type::plugin && (! item.plugin || ! item.plugin->instance())) continue;
        if (i < partition.size())
            stageLoad[(size_t) juce::jlimit (0, 7, partition[i])] += getItemLoad (item);
        ++i;
    }
    return *std::max_element (stageLoad.begin(), stageLoad.end());
}

float HostEngine::getBusiestCoreLoad() const
{
    if (pipelineStages <= 1 || currentPartition.empty())
    {
        float total = 0;
        for (auto& item : items) total += getItemLoad (item);
        return total;
    }
    return busiestStageLoad (currentPartition);
}

void HostEngine::timerCallback()
{
    // Plugins can change their latency (e.g. a lookahead switch): rebuild so splits stay aligned.
    if (computeChainLatency() != lastPlanLatency)
        rewire();

    // After a structural change, re-balance the pipeline once the new plugins' CPU has been
    // measured. Only rebuild if the split between cores actually changes (a rebuild restarts
    // the pipeline, which costs one block of audio).
    if (rebalanceAt > 0 && juce::Time::getMillisecondCounterHiRes() > rebalanceAt)
    {
        rebalanceAt = 0;
        const int numStages = juce::jlimit (1, juce::jmax (1, (int) items.size()), pipelineStages);
        if (numStages > 1)
        {
            auto candidate = computePartition (numStages);
            // Hysteresis: only worth a restart if the busiest core gets noticeably lighter.
            if (candidate != currentPartition && candidate.size() == currentPartition.size()
                 && busiestStageLoad (candidate) < 0.85f * busiestStageLoad (currentPartition))
                rewire();
        }
    }
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
    const double sr = rack->getCurrentSampleRate();
    const int bs = rack->getCurrentBlockSize();

    const std::string name = desc.name.toStdString();
    std::unique_ptr<juce::AudioPluginInstance> inst;
    {
        CrashGuard::Scope guard (name.c_str(), "loading");
        DBG ("PEQUOD: loading " << desc.name);
        inst = formatManager.createPluginInstance (desc, sr, bs, error);
    }
    if (inst == nullptr)
    {
        if (error.isEmpty()) error = "Could not load " + desc.name;
        return {};
    }

    // Prefer stereo main buses; plugins that refuse keep their own layout (mono is handled when rendering).
    for (bool isInput : { true, false })
        if (auto* bus = inst->getBus (isInput, 0))
            if (bus->getNumberOfChannels() != 2)
                bus->setCurrentLayout (juce::AudioChannelSet::stereo());

    {
        CrashGuard::Scope guard (name.c_str(), "starting");
        rack->preparePlugin (*inst);
    }

    auto slot = std::make_shared<PluginSlot>();
    slot->nameUtf8 = name;
    slot->uid = nextUid++;
    slot->stableId = newStableId();
    slot->description = desc;
    slot->plugin = std::move (inst);
    attachListener (*slot);
    return slot;
}

void HostEngine::destroySlot (const std::shared_ptr<PluginSlot>& slot, bool notify)
{
    // Callers must already have rebuilt the plan without this slot.
    if (slot == nullptr) return;

    if (notify) detachFromUi (*slot);

    if (auto* inst = slot->instance())
    {
        CrashGuard::Scope guard (slot->nameUtf8.c_str(), "closing");
        inst->releaseResources();
    }

    slot->plugin.reset();
}

void HostEngine::detachFromUi (PluginSlot& slot)
{
    if (onSlotAboutToBeRemoved)
        onSlotAboutToBeRemoved (slot.uid);

    if (auto* inst = slot.instance())
        if (slot.listener != nullptr)
            for (auto* p : inst->getParameters())
                p->removeListener (slot.listener.get());
    slot.listener.reset();
}

void HostEngine::retireSlot (const std::shared_ptr<PluginSlot>& slot)
{
    // Callers must already have rebuilt the plan without this slot.
    // Kept loaded (not processing) so going back to a preset that uses it is instant.
    if (slot == nullptr || slot->instance() == nullptr) return;
    detachFromUi (*slot);
    if (slot->crashed) { destroySlot (slot, false); return; }

    spares.push_back (slot);
    constexpr size_t maxSpares = 12;
    while (spares.size() > maxSpares)
    {
        destroySlot (spares.front(), false);
        spares.erase (spares.begin());
    }
}

//==============================================================================
int HostEngine::addPlugin (const juce::PluginDescription& desc, RackLocation where, juce::String& errorOut)
{
    aboutToEdit();
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
    aboutToEdit();
    RackItem item;
    item.uid = nextUid++;
    item.type = RackItem::Type::split;
    item.stableId = newStableId();
    item.state = std::make_shared<SplitState>();

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
    aboutToEdit();
    if (auto s = detachSlot (slotUid))
    {
        rewire();            // audio thread stops using it...
        retireSlot (s);      // ...before it is put aside (kept loaded, so undo is instant)
        changed();
    }
}

void HostEngine::removeItem (int itemUid)
{
    aboutToEdit();
    for (auto it = items.begin(); it != items.end(); ++it)
    {
        if (it->uid != itemUid) continue;

        auto item = std::move (*it);
        items.erase (it);
        rewire();

        if (item.type == RackItem::Type::plugin)
            retireSlot (item.plugin);
        else
            for (auto& lane : item.lanes)
                for (auto& s : lane)
                    retireSlot (s);

        changed();
        return;
    }
}

void HostEngine::moveSlot (int slotUid, RackLocation where)
{
    aboutToEdit();
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
    aboutToEdit();
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
    auto old = std::move (items);
    items.clear();
    clearModifiers();
    scenes.clear();
    currentScene = -1;
    inputStage->presetDb = 0.0f;
    outputStage->presetDb = 0.0f;
    if (onScenesChanged) onScenesChanged();
    rewire();

    for (auto& item : old)
    {
        if (item.type == RackItem::Type::plugin)
            retireSlot (item.plugin);
        else
            for (auto& lane : item.lanes)
                for (auto& s : lane)
                    retireSlot (s);
    }
    changed();
}

void HostEngine::setBypassed (int slotUid, bool shouldBypass)
{
    if (auto s = findSlot (slotUid))
    {
        s->bypassed = shouldBypass;
        changed();
    }
}

void HostEngine::setSplitMix (int itemUid, float mix)
{
    if (auto* item = findItem (itemUid))
    {
        item->mix = juce::jlimit (0.0f, 1.0f, mix);
        if (item->state != nullptr)
            item->state->mix = item->mix;
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

int HostEngine::findUidByStableId (const juce::String& stableId, bool isSplit) const
{
    if (stableId.isEmpty()) return 0;
    for (auto& item : items)
    {
        if (isSplit)
        {
            if (item.type == RackItem::Type::split && item.stableId == stableId) return item.uid;
            continue;
        }
        if (item.type == RackItem::Type::plugin && item.plugin && item.plugin->stableId == stableId)
            return item.plugin->uid;
        for (auto& lane : item.lanes)
            for (auto& sl : lane)
                if (sl->stableId == stableId)
                    return sl->uid;
    }
    return 0;
}

juce::String HostEngine::getStableId (int uid, bool isSplit) const
{
    for (auto& item : items)
    {
        if (isSplit)
        {
            if (item.uid == uid && item.type == RackItem::Type::split) return item.stableId;
            continue;
        }
        if (item.type == RackItem::Type::plugin && item.plugin && item.plugin->uid == uid) return item.plugin->stableId;
        for (auto& lane : item.lanes)
            for (auto& sl : lane)
                if (sl->uid == uid) return sl->stableId;
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
    x->setAttribute ("id", s.stableId);
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
    // the preset's own levels (the master levels aren't saved with presets)
    root->setAttribute ("inGain", inputStage->presetDb.load());
    root->setAttribute ("outGain", outputStage->presetDb.load());
    if (! scenes.empty())
        root->addChildElement (scenesToXml().release());
    root->setAttribute ("inputMode", inputStage->inputMode.load());
    root->setAttribute ("bpm", transport.bpm.load());
    root->setAttribute ("input", rack->instrumentInput.load() ? "instrument" : "guitar");
    if (! modifiers.empty())
        root->addChildElement (modifiersToXml().release());

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
            xi->setAttribute ("sid", item.stableId);
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

    // Switching presets without a gap:
    //  1. plugins this preset shares with the one playing are kept loaded and just get new settings;
    //  2. other plugins come from the spare pool (loaded earlier) or are loaded now, while the old
    //     preset keeps playing;
    //  3. only then is the output faded out for a few ms, the new chain swapped in and faded back in.
    std::vector<std::shared_ptr<PluginSlot>> playing;   // in the current rack, still processing
    for (auto& item : items)
    {
        if (item.type == RackItem::Type::plugin) { if (item.plugin) playing.push_back (item.plugin); }
        else for (auto& lane : item.lanes) for (auto& sl : lane) playing.push_back (sl);
    }

    struct Pending
    {
        std::shared_ptr<PluginSlot> slot;
        juce::MemoryBlock state;
        bool hasState = false;
        bool live = false;          // still processing in the old rack: settings applied after the fade
        bool bypass = false;
        juce::String stableId;
    };
    std::vector<Pending> pending;

    auto takeMatching = [] (std::vector<std::shared_ptr<PluginSlot>>& from, const juce::PluginDescription& desc)
    {
        for (auto it = from.begin(); it != from.end(); ++it)
            if ((*it)->instance() != nullptr && ! (*it)->crashed && (*it)->description.isDuplicateOf (desc))
            {
                auto sl = *it;
                from.erase (it);
                return sl;
            }
        return std::shared_ptr<PluginSlot>();
    };

    auto makeSlot = [&] (const juce::XmlElement& xs) -> std::shared_ptr<PluginSlot>
    {
        juce::PluginDescription desc;
        if (auto* xd = xs.getChildByName ("PLUGIN"))
            desc.loadFromXml (*xd);

        // Prefer the locally-scanned description (path may differ between machines).
        for (auto& known : knownPlugins.getTypes())
            if (known.isDuplicateOf (desc)) { desc = known; break; }

        // A PEQUOD plugin saved as a VST3 runs on the built-in engine (it reads the VST3's settings).
        if (desc.pluginFormatName != PequodNative::formatName && desc.manufacturerName.startsWith ("PEQUOD"))
            for (auto& i : PequodNative::all())
                if (i.name == desc.name && i.vendor == desc.manufacturerName) { desc = PequodNative::describe (i); break; }

        if (skipPlugins.contains (desc.name, true))
        {
            errors.add (desc.name + ": left out (it crashed last time)");
            return {};
        }

        Pending p;
        bool fromSpares = false;
        p.slot = takeMatching (playing, desc);
        p.live = p.slot != nullptr;
        if (p.slot == nullptr && (p.slot = takeMatching (spares, desc)) != nullptr)
            fromSpares = true;

        if (p.slot == nullptr)
        {
            juce::String err;
            p.slot = createSlot (desc, err);
            if (p.slot == nullptr)
            {
                errors.add (desc.name + ": " + err);
                return {};
            }
        }
        else
        {
            if (fromSpares)
            {
                CrashGuard::Scope guard (p.slot->nameUtf8.c_str(), "starting");
                rack->preparePlugin (*p.slot->instance());   // the device may have changed since
            }
            if (fromSpares) p.slot->uid = nextUid++;          // a fresh module as far as the UI is concerned
        }

        if (auto* xst = xs.getChildByName ("STATE"))
            p.hasState = p.state.fromBase64Encoding (xst->getAllSubText().trim());

        // Not playing: safe to set up right away.
        if (! p.live)
        {
            if (p.hasState)
            {
                CrashGuard::Scope guard (p.slot->nameUtf8.c_str(), "loading its settings");
                p.slot->instance()->setStateInformation (p.state.getData(), (int) p.state.getSize());
            }
            if (fromSpares) attachListener (*p.slot);
        }

        p.bypass = xs.getBoolAttribute ("bypass", false);
        p.stableId = xs.hasAttribute ("id") ? xs.getStringAttribute ("id") : newStableId();
        if (! p.live)
        {
            p.slot->bypassed = p.bypass;
            p.slot->stableId = p.stableId;
        }
        pending.push_back (p);
        return p.slot;
    };

    std::vector<RackItem> newItems;
    for (auto* xi : root.getChildWithTagNameIterator ("ITEM"))
    {
        RackItem item;
        item.uid = nextUid++;

        if (xi->getStringAttribute ("type") == "split")
        {
            item.type = RackItem::Type::split;
            item.stableId = xi->getStringAttribute ("sid", newStableId());
            item.mix = (float) xi->getDoubleAttribute ("mix", 0.5);
            item.state = std::make_shared<SplitState>();
            item.state->mix = item.mix;

            for (auto* xl : xi->getChildWithTagNameIterator ("LANE"))
            {
                auto l = juce::jlimit (0, 1, xl->getIntAttribute ("id"));
                for (auto* xs : xl->getChildWithTagNameIterator ("SLOT"))
                    if (auto sl = makeSlot (*xs))
                        item.lanes[l].push_back (sl);
            }
            newItems.push_back (std::move (item));
        }
        else if (auto* xs = xi->getChildByName ("SLOT"))
        {
            if (auto sl = makeSlot (*xs))
            {
                item.type = RackItem::Type::plugin;
                item.plugin = sl;
                newItems.push_back (std::move (item));
            }
        }
    }

    // Everything is loaded: now the short swap.
    const auto swapStart = juce::Time::getMillisecondCounterHiRes();
    rack->fadeOut();
    const bool faded = rack->waitForSilence();

    auto oldItems = std::move (items);
    items.clear();
    rewire();                                    // nothing processing now

    for (auto& p : pending)
        if (p.live)
        {
            detachFromUi (*p.slot);              // closes its window etc. under the old uid
            p.slot->uid = nextUid++;             // a fresh module as far as the UI is concerned
            p.slot->bypassed = p.bypass;
            p.slot->stableId = p.stableId;
            if (p.hasState)
            {
                CrashGuard::Scope guard (p.slot->nameUtf8.c_str(), "loading its settings");
                p.slot->instance()->setStateInformation (p.state.getData(), (int) p.state.getSize());
            }
            p.slot->instance()->reset();         // drop the old preset's echoes / reverb
        }
    for (auto& p : pending)
        if (p.live || p.slot->listener == nullptr)
            attachListener (*p.slot);

    currentPresetName = root.getStringAttribute ("name", "Untitled");
    inputStage->presetDb  = (float) root.getDoubleAttribute ("inGain", 0.0);
    outputStage->presetDb = (float) root.getDoubleAttribute ("outGain", 0.0);
    if (root.hasAttribute ("bpm")) transport.setBpm (root.getDoubleAttribute ("bpm", 120.0));   // older presets keep the current tempo
    inputStage->inputMode = root.getIntAttribute ("inputMode", (int) GainMeterProcessor::InputMode::Left);
    rack->instrumentInput = root.getStringAttribute ("input") == "instrument";
    clearModifiers();
    if (auto* xm = root.getChildByName ("MODIFIERS"))
        modifiersFromXml (*xm);
    scenes.clear();
    currentScene = -1;
    if (auto* xs = root.getChildByName ("SCENES"))
        scenesFromXml (*xs);
    if (onScenesChanged) onScenesChanged();

    items = std::move (newItems);
    rewire();
    rack->fadeIn();
    DBG ("PEQUOD: preset swap " << (int) (juce::Time::getMillisecondCounterHiRes() - swapStart) << " ms"
         << (faded ? "" : " (audio not running)"));

    // Plugins of the old preset that this one doesn't use: kept loaded as spares.
    for (auto& sl : playing)
        retireSlot (sl);
    oldItems.clear();

    changed();
    return errors.isEmpty();
}

//==============================================================================
juce::String HostEngine::modifierName (int i) const
{
    if (! juce::isPositiveAndBelow (i, (int) modifiers.size())) return {};
    const auto type = rack->modBank[modifiers[(size_t) i].bank].type.load();
    int number = 1;
    for (int k = 0; k < i; ++k)
        if (rack->modBank[modifiers[(size_t) k].bank].type.load() == type) ++number;
    juce::String base;
    switch ((Modifiers::Type) type)
    {
        case Modifiers::Type::lfo:       base = "LFO"; break;
        case Modifiers::Type::follower:  base = "FOLLOWER"; break;
        case Modifiers::Type::envelope:  base = "ENVELOPE"; break;
        case Modifiers::Type::sequencer: base = "STEPS"; break;
        case Modifiers::Type::macro:     base = "MACRO"; break;
        case Modifiers::Type::pitch:     base = "PITCH"; break;
        case Modifiers::Type::random:    base = "RANDOM"; break;
        case Modifiers::Type::ducker:    base = "DUCKER"; break;
        case Modifiers::Type::swell:     base = "SWELL"; break;
        case Modifiers::Type::counter:   base = "STEPPER"; break;
    }
    return base + " " + juce::String (number);
}

int HostEngine::addModifier (Modifiers::Type type)
{
    for (int b = 0; b < Modifiers::maxModifiers; ++b)
    {
        bool used = false;
        for (auto& m : modifiers) used = used || m.bank == b;
        if (used) continue;

        aboutToEdit();
        auto& slot = rack->modBank[b];
        slot.resetSettings();
        if (type == Modifiers::Type::envelope || type == Modifiers::Type::counter) slot.trigger = Modifiers::Slot::triggerPicking;
        if (type == Modifiers::Type::ducker) { slot.attackMs = 5.0f; slot.releaseMs = 600.0f; }
        if (type == Modifiers::Type::swell)  { slot.aMs = 1500.0f; slot.rMs = 1500.0f; }
        if (type == Modifiers::Type::random) { slot.sync = false; slot.rateHz = 0.5f; }
        slot.type = (int) type;
        ModifierInfo info;
        info.bank = b;
        info.id = juce::Uuid().toString().substring (0, 8);
        modifiers.push_back (info);
        rewire();
        changed();
        if (onModifiersChanged) onModifiersChanged();
        return (int) modifiers.size() - 1;
    }
    return -1;
}

void HostEngine::removeModifier (int i)
{
    if (! juce::isPositiveAndBelow (i, (int) modifiers.size())) return;
    aboutToEdit();
    const int bank = modifiers[(size_t) i].bank;
    modifiers.erase (modifiers.begin() + i);
    rewire();                                   // the audio thread stops using it...
    rack->modBank[bank].type = 0;               // ...then the slot is free
    if (modLearn.load() == i) modLearn = -1;
    changed();
    if (onModifiersChanged) onModifiersChanged();
}

void HostEngine::moveModifier (int from, int to)
{
    if (! juce::isPositiveAndBelow (from, (int) modifiers.size()) || ! juce::isPositiveAndBelow (to, (int) modifiers.size()) || from == to) return;
    aboutToEdit();
    auto m = modifiers[(size_t) from];
    modifiers.erase (modifiers.begin() + from);
    modifiers.insert (modifiers.begin() + to, m);
    rewire();
    changed();
    if (onModifiersChanged) onModifiersChanged();
}

void HostEngine::removeModAssignment (int i, int a)
{
    if (! juce::isPositiveAndBelow (i, (int) modifiers.size())) return;
    auto& list = modifiers[(size_t) i].assignments;
    if (! juce::isPositiveAndBelow (a, (int) list.size())) return;
    // Put the knob back where it started.
    auto assignment = list[(size_t) a];
    list.erase (list.begin() + a);
    rewire();
    for (auto& sl : getFlatSlots())
        if (sl->stableId == assignment->slotId && sl->instance() != nullptr)
            if (auto* p = sl->instance()->getParameters()[assignment->paramIndex])
                p->setValueNotifyingHost (assignment->from.load());
    changed();
    if (onModifiersChanged) onModifiersChanged();
}

void HostEngine::modifierParamTouched (int slotUid, int paramIndex, float value)
{
    const int i = modLearn.load();
    if (! juce::isPositiveAndBelow (i, (int) modifiers.size())) return;
    auto slot = findSlot (slotUid);
    if (slot == nullptr) return;
    modLearn = -1;

    auto& list = modifiers[(size_t) i].assignments;
    for (auto& a : list)
        if (a->slotId == slot->stableId && a->paramIndex == paramIndex)
        {
            changed();
            if (onModifiersChanged) onModifiersChanged();
            return;                                         // already moved by this modifier
        }

    // From where the knob is now, sweeping halfway across its range.
    auto a = std::make_shared<Modifiers::Assignment>();
    a->slotId = slot->stableId;
    a->paramIndex = paramIndex;
    a->from = value;
    a->to = value < 0.5f ? juce::jmin (1.0f, value + 0.5f) : juce::jmax (0.0f, value - 0.5f);
    list.push_back (a);
    rewire();
    markModified();
    changed();
    if (onModifiersChanged) onModifiersChanged();
}

void HostEngine::modifierKnobTouched (int target, int modParam, float value)
{
    const int i = modLearn.load();
    if (! juce::isPositiveAndBelow (i, (int) modifiers.size()) || ! juce::isPositiveAndBelow (target, (int) modifiers.size()) || i == target) return;
    modLearn = -1;
    const auto slotId = juce::String (Modifiers::Assignment::modPrefix) + modifiers[(size_t) target].id;
    auto& list = modifiers[(size_t) i].assignments;
    for (auto& a : list)
        if (a->slotId == slotId && a->paramIndex == modParam) { changed(); if (onModifiersChanged) onModifiersChanged(); return; }

    auto a = std::make_shared<Modifiers::Assignment>();
    a->slotId = slotId;
    a->paramIndex = modParam;
    a->from = value;
    a->to = value < 0.5f ? juce::jmin (1.0f, value + 0.5f) : juce::jmax (0.0f, value - 0.5f);
    list.push_back (a);
    rewire();
    markModified();
    changed();
    if (onModifiersChanged) onModifiersChanged();
}

juce::String HostEngine::describeAssignment (const Modifiers::Assignment& a) const
{
    if (a.targetsModifier())
    {
        const auto id = a.slotId.fromFirstOccurrenceOf (Modifiers::Assignment::modPrefix, false, false);
        for (int k = 0; k < (int) modifiers.size(); ++k)
            if (modifiers[(size_t) k].id == id)
                return modifierName (k) + (a.paramIndex == Modifiers::modParamRate ? "  RATE"
                                         : a.paramIndex == Modifiers::modParamAmount ? "  AMOUNT" : "  KNOB");
        return "(modifier removed)";
    }
    for (auto& sl : getFlatSlots())
        if (sl->stableId == a.slotId && sl->instance() != nullptr)
        {
            auto* p = sl->instance()->getParameters()[a.paramIndex];
            return sl->description.name + "  " + (p != nullptr ? p->getName (40) : juce::String ("?"));
        }
    return "(effect not in this preset)";
}

void HostEngine::clearModifiers()
{
    std::vector<int> banks;
    for (auto& m : modifiers) banks.push_back (m.bank);
    modifiers.clear();
    modLearn = -1;
    rewire();
    for (auto b : banks) rack->modBank[b].type = 0;
    if (onModifiersChanged) onModifiersChanged();
}

std::unique_ptr<juce::XmlElement> HostEngine::modifiersToXml() const
{
    auto x = std::make_unique<juce::XmlElement> ("MODIFIERS");
    for (auto& m : modifiers)
    {
        auto& s = rack->modBank[m.bank];
        auto* xm = x->createNewChildElement ("MOD");
        xm->setAttribute ("type", s.type.load());
        xm->setAttribute ("on", s.on.load());
        xm->setAttribute ("amount", s.amount.load());
        xm->setAttribute ("invert", s.invert.load());
        xm->setAttribute ("shape", s.shape.load());
        xm->setAttribute ("sync", s.sync.load());
        xm->setAttribute ("division", s.division.load());
        xm->setAttribute ("rate", s.rateHz.load());
        xm->setAttribute ("phase", s.phase.load());
        xm->setAttribute ("sens", s.sensitivity.load());
        xm->setAttribute ("attack", s.attackMs.load());
        xm->setAttribute ("release", s.releaseMs.load());
        xm->setAttribute ("trigger", s.trigger.load());
        xm->setAttribute ("hold", s.holdMode.load());
        xm->setAttribute ("a", s.aMs.load());
        xm->setAttribute ("d", s.dMs.load());
        xm->setAttribute ("s", s.sustain.load());
        xm->setAttribute ("r", s.rMs.load());
        xm->setAttribute ("pick", s.pickThresholdDb.load());
        juce::StringArray steps;
        for (int k = 0; k < Modifiers::maxSteps; ++k) steps.add (juce::String (s.steps[k].load(), 3));
        xm->setAttribute ("steps", steps.joinIntoString (" "));
        xm->setAttribute ("length", s.length.load());
        xm->setAttribute ("glide", s.glide.load());
        xm->setAttribute ("id", m.id);
        xm->setAttribute ("macro", s.macroValue.load());
        xm->setAttribute ("lowNote", s.lowNote.load());
        xm->setAttribute ("highNote", s.highNote.load());
        xm->setAttribute ("smooth", s.glideMs.load());
        xm->setAttribute ("randomMode", s.randomMode.load());
        xm->setAttribute ("threshold", s.thresholdDb.load());
        xm->setAttribute ("holdMs", s.holdMs.load());
        for (auto& a : m.assignments)
        {
            auto* xa = xm->createNewChildElement ("TO");
            xa->setAttribute ("slot", a->slotId);
            xa->setAttribute ("param", a->paramIndex);
            xa->setAttribute ("from", a->from.load());
            xa->setAttribute ("to", a->to.load());
            xa->setAttribute ("curve", a->curve.load());
        }
    }
    return x;
}

void HostEngine::modifiersFromXml (const juce::XmlElement& x)
{
    for (auto* xm : x.getChildWithTagNameIterator ("MOD"))
    {
        const int type = xm->getIntAttribute ("type");
        if (type < 1 || type > Modifiers::numTypes) continue;
        int bank = -1;
        for (int b = 0; b < Modifiers::maxModifiers && bank < 0; ++b)
        {
            bool used = false;
            for (auto& m : modifiers) used = used || m.bank == b;
            if (! used) bank = b;
        }
        if (bank < 0) break;

        auto& s = rack->modBank[bank];
        s.resetSettings();
        s.on = xm->getBoolAttribute ("on", true);
        s.amount = (float) xm->getDoubleAttribute ("amount", 1.0);
        s.invert = xm->getBoolAttribute ("invert", false);
        s.shape = xm->getIntAttribute ("shape", 0);
        s.sync = xm->getBoolAttribute ("sync", true);
        s.division = xm->getIntAttribute ("division", Modifiers::defaultDivision());
        s.rateHz = (float) xm->getDoubleAttribute ("rate", 1.0);
        s.phase = (float) xm->getDoubleAttribute ("phase", 0.0);
        s.sensitivity = (float) xm->getDoubleAttribute ("sens", 20.0);
        s.attackMs = (float) xm->getDoubleAttribute ("attack", 10.0);
        s.releaseMs = (float) xm->getDoubleAttribute ("release", 150.0);
        s.trigger = xm->getIntAttribute ("trigger", 0);
        s.holdMode = xm->getBoolAttribute ("hold", false);
        s.aMs = (float) xm->getDoubleAttribute ("a", 5.0);
        s.dMs = (float) xm->getDoubleAttribute ("d", 300.0);
        s.sustain = (float) xm->getDoubleAttribute ("s", 0.0);
        s.rMs = (float) xm->getDoubleAttribute ("r", 400.0);
        s.pickThresholdDb = (float) xm->getDoubleAttribute ("pick", -40.0);
        auto steps = juce::StringArray::fromTokens (xm->getStringAttribute ("steps"), " ", {});
        for (int k = 0; k < juce::jmin (steps.size(), Modifiers::maxSteps); ++k) s.steps[k] = steps[k].getFloatValue();
        s.length = xm->getIntAttribute ("length", 8);
        s.glide = (float) xm->getDoubleAttribute ("glide", 0.0);
        s.macroValue = (float) xm->getDoubleAttribute ("macro", 0.0);
        s.lowNote = (float) xm->getDoubleAttribute ("lowNote", 40.0);
        s.highNote = (float) xm->getDoubleAttribute ("highNote", 88.0);
        s.glideMs = (float) xm->getDoubleAttribute ("smooth", 80.0);
        s.randomMode = xm->getIntAttribute ("randomMode", 0);
        s.thresholdDb = (float) xm->getDoubleAttribute ("threshold", -45.0);
        s.holdMs = (float) xm->getDoubleAttribute ("holdMs", 150.0);
        s.type = type;

        ModifierInfo info;
        info.bank = bank;
        info.id = xm->getStringAttribute ("id", juce::Uuid().toString().substring (0, 8));
        for (auto* xa : xm->getChildWithTagNameIterator ("TO"))
        {
            auto a = std::make_shared<Modifiers::Assignment>();
            a->slotId = xa->getStringAttribute ("slot");
            a->paramIndex = xa->getIntAttribute ("param", -1);
            a->from = (float) xa->getDoubleAttribute ("from", 0.0);
            a->to = (float) xa->getDoubleAttribute ("to", 1.0);
            a->curve = juce::jlimit (0, Modifiers::numCurves - 1, xa->getIntAttribute ("curve", 0));
            info.assignments.push_back (a);
        }
        modifiers.push_back (info);
    }
    if (onModifiersChanged) onModifiersChanged();
}

//==============================================================================
HostEngine::Scene HostEngine::captureScene (const juce::String& name) const
{
    Scene sc;
    sc.name = name;
    for (auto& sl : getFlatSlots())
    {
        Scene::SlotState st;
        st.bypass = sl->bypassed.load();
        if (auto* inst = sl->instance())
        {
            for (auto* p : inst->getParameters())
                st.params.push_back (p->getValue());
            CrashGuard::Scope guard (sl->nameUtf8.c_str(), "saving its settings");
            inst->getStateInformation (st.state);
        }
        sc.slots[sl->stableId] = std::move (st);
    }
    for (auto& item : items)
        if (item.type == RackItem::Type::split)
            sc.splitMix[item.stableId] = item.mix;
    for (auto& m : modifiers)
        sc.modifiersOn[m.id] = rack->modBank[m.bank].on.load();
    sc.presetIn = inputStage->presetDb.load();
    sc.presetOut = outputStage->presetDb.load();
    return sc;
}

void HostEngine::applyScene (const Scene& sc)
{
    // Only what differs is changed, so knobs that are the same don't jump.
    for (auto& sl : getFlatSlots())
    {
        auto it = sc.slots.find (sl->stableId);
        if (it == sc.slots.end()) continue;          // added after the scene was made: left as it is
        sl->bypassed = it->second.bypass;
        if (auto* inst = sl->instance())
        {
            auto& params = inst->getParameters();
            for (int k = 0; k < juce::jmin (params.size(), (int) it->second.params.size()); ++k)
            {
                const float v = it->second.params[(size_t) k];
                if (std::abs (params[k]->getValue() - v) > 1.0e-6f)
                    params[k]->setValueNotifyingHost (v);
            }
        }
    }
    for (auto& item : items)
        if (item.type == RackItem::Type::split)
        {
            auto it = sc.splitMix.find (item.stableId);
            if (it != sc.splitMix.end())
            {
                item.mix = it->second;
                if (item.state != nullptr) item.state->mix = it->second;
            }
        }
    for (auto& m : modifiers)
    {
        auto it = sc.modifiersOn.find (m.id);
        if (it != sc.modifiersOn.end()) rack->modBank[m.bank].on = it->second;
    }
    inputStage->presetDb = sc.presetIn;
    outputStage->presetDb = sc.presetOut;

    // A moment later (once the plugins have taken the new knob values), check the rest of each
    // plugin's state and restore it where it differs.
    const int serial = ++sceneSerial;
    const int index = currentScene;
    std::weak_ptr<int> alive = lifeToken;
    juce::Timer::callAfterDelay (60, [this, alive, serial, index]
    {
        if (alive.expired() || serial != sceneSerial) return;    // gone, or switched again since
        finishScene (index);
    });
}

void HostEngine::finishScene (int index)
{
    if (! juce::isPositiveAndBelow (index, (int) scenes.size()) || index != currentScene) return;
    const auto& sc = scenes[(size_t) index];

    std::vector<std::pair<std::shared_ptr<PluginSlot>, const juce::MemoryBlock*>> restore;
    for (auto& sl : getFlatSlots())
    {
        auto it = sc.slots.find (sl->stableId);
        if (it == sc.slots.end() || it->second.state.isEmpty() || sl->instance() == nullptr) continue;
        juce::MemoryBlock now;
        {
            CrashGuard::Scope guard (sl->nameUtf8.c_str(), "saving its settings");
            sl->instance()->getStateInformation (now);
        }
        if (now != it->second.state)
            restore.emplace_back (sl, &it->second.state);
    }
    DBG ("PEQUOD: scene " << index << ": " << (int) restore.size() << " plugin(s) need their full state");
    if (restore.empty()) return;

    // Something that isn't a knob differs (an IR, a model...): restore those plugins under a short fade.
    rack->fadeOut();
    rack->waitForSilence (40);
    for (auto& [sl, state] : restore)
    {
        CrashGuard::Scope guard (sl->nameUtf8.c_str(), "loading its settings");
        sl->instance()->setStateInformation (state->getData(), (int) state->getSize());
    }
    rack->fadeIn();
    changed();
}

int HostEngine::addScene()
{
    if ((int) scenes.size() >= maxScenes) return -1;
    aboutToEdit();
    if (scenes.empty())
    {
        scenes.push_back (captureScene ("SCENE 1"));
        currentScene = 0;
    }
    else if (juce::isPositiveAndBelow (currentScene, (int) scenes.size()))
    {
        const auto name = scenes[(size_t) currentScene].name;
        scenes[(size_t) currentScene] = captureScene (name);        // keep what was changed
    }
    // the new one starts as a copy of how it sounds now
    scenes.push_back (captureScene ("SCENE " + juce::String ((int) scenes.size() + 1)));
    currentScene = (int) scenes.size() - 1;
    markModified();
    changed();
    if (onScenesChanged) onScenesChanged();
    return currentScene;
}

void HostEngine::selectScene (int i)
{
    if (! juce::isPositiveAndBelow (i, (int) scenes.size()) || i == currentScene) return;
    if (juce::isPositiveAndBelow (currentScene, (int) scenes.size()))
    {
        const auto name = scenes[(size_t) currentScene].name;
        scenes[(size_t) currentScene] = captureScene (name);        // changes made in it stay with it
    }
    currentScene = i;
    applyScene (scenes[(size_t) i]);
    changed();
    if (onScenesChanged) onScenesChanged();
}

void HostEngine::renameScene (int i, const juce::String& name)
{
    if (! juce::isPositiveAndBelow (i, (int) scenes.size())) return;
    scenes[(size_t) i].name = name.trim().toUpperCase().substring (0, 20);
    markModified();
    changed();
    if (onScenesChanged) onScenesChanged();
}

void HostEngine::removeScene (int i)
{
    if (! juce::isPositiveAndBelow (i, (int) scenes.size())) return;
    aboutToEdit();
    const bool wasCurrent = i == currentScene;
    scenes.erase (scenes.begin() + i);
    if (scenes.empty()) currentScene = -1;
    else if (wasCurrent) { currentScene = juce::jmin (i, (int) scenes.size() - 1); applyScene (scenes[(size_t) currentScene]); }
    else if (currentScene > i) --currentScene;
    markModified();
    changed();
    if (onScenesChanged) onScenesChanged();
}

void HostEngine::moveScene (int from, int to)
{
    if (! juce::isPositiveAndBelow (from, (int) scenes.size()) || ! juce::isPositiveAndBelow (to, (int) scenes.size()) || from == to) return;
    auto sc = scenes[(size_t) from];
    scenes.erase (scenes.begin() + from);
    scenes.insert (scenes.begin() + to, sc);
    if (currentScene == from) currentScene = to;
    else if (from < currentScene && to >= currentScene) --currentScene;
    else if (from > currentScene && to <= currentScene) ++currentScene;
    markModified();
    changed();
    if (onScenesChanged) onScenesChanged();
}

std::unique_ptr<juce::XmlElement> HostEngine::scenesToXml() const
{
    auto x = std::make_unique<juce::XmlElement> ("SCENES");
    x->setAttribute ("current", currentScene);
    for (int i = 0; i < (int) scenes.size(); ++i)
    {
        // the chosen scene is saved as it is now (including changes not yet stored in it)
        const Scene sc = i == currentScene ? captureScene (scenes[(size_t) i].name) : scenes[(size_t) i];
        auto* xs = x->createNewChildElement ("SCENE");
        xs->setAttribute ("name", sc.name);
        xs->setAttribute ("in", sc.presetIn);
        xs->setAttribute ("out", sc.presetOut);
        for (auto& [id, st] : sc.slots)
        {
            auto* e = xs->createNewChildElement ("SLOT");
            e->setAttribute ("id", id);
            e->setAttribute ("bypass", st.bypass);
            juce::MemoryBlock mb (st.params.data(), st.params.size() * sizeof (float));
            e->setAttribute ("params", mb.toBase64Encoding());
            if (! st.state.isEmpty())
                e->setAttribute ("state", st.state.toBase64Encoding());
        }
        for (auto& [id, mix] : sc.splitMix)
        {
            auto* e = xs->createNewChildElement ("SPLIT");
            e->setAttribute ("id", id);
            e->setAttribute ("mix", mix);
        }
        for (auto& [id, on] : sc.modifiersOn)
        {
            auto* e = xs->createNewChildElement ("MOD");
            e->setAttribute ("id", id);
            e->setAttribute ("on", on);
        }
    }
    return x;
}

void HostEngine::scenesFromXml (const juce::XmlElement& x)
{
    for (auto* xs : x.getChildWithTagNameIterator ("SCENE"))
    {
        if ((int) scenes.size() >= maxScenes) break;
        Scene sc;
        sc.name = xs->getStringAttribute ("name", "SCENE " + juce::String ((int) scenes.size() + 1));
        sc.presetIn = (float) xs->getDoubleAttribute ("in", 0.0);
        sc.presetOut = (float) xs->getDoubleAttribute ("out", 0.0);
        for (auto* e : xs->getChildWithTagNameIterator ("SLOT"))
        {
            Scene::SlotState st;
            st.bypass = e->getBoolAttribute ("bypass", false);
            juce::MemoryBlock mb;
            if (mb.fromBase64Encoding (e->getStringAttribute ("params")))
            {
                st.params.resize (mb.getSize() / sizeof (float));
                std::memcpy (st.params.data(), mb.getData(), st.params.size() * sizeof (float));
            }
            if (e->hasAttribute ("state"))
                st.state.fromBase64Encoding (e->getStringAttribute ("state"));
            sc.slots[e->getStringAttribute ("id")] = std::move (st);
        }
        for (auto* e : xs->getChildWithTagNameIterator ("SPLIT"))
            sc.splitMix[e->getStringAttribute ("id")] = (float) e->getDoubleAttribute ("mix", 0.5);
        for (auto* e : xs->getChildWithTagNameIterator ("MOD"))
            sc.modifiersOn[e->getStringAttribute ("id")] = e->getBoolAttribute ("on", true);
        scenes.push_back (std::move (sc));
    }
    // The plugins were saved as the chosen scene sounded, so nothing needs applying.
    currentScene = scenes.empty() ? -1 : juce::jlimit (0, (int) scenes.size() - 1, x.getIntAttribute ("current", 0));
}
