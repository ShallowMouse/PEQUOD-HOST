#include "RackProcessor.h"
#include "Transport.h"
#include "HostEngine.h"

namespace
{
    int defaultWorkerCount()
    {
        // Leave one core for the audio thread itself (and one spare on big machines).
        const int cores = juce::SystemStats::getNumCpus();
        return juce::jlimit (0, 7, cores - (cores > 4 ? 2 : 1));
    }

    struct LaneJob
    {
        RackProcessor* rack;
        RenderLane* lane;
        int n;

        static void run (void* p)
        {
            auto& j = *static_cast<LaneJob*> (p);
            for (auto& rp : j.lane->plugins)
                j.rack->processPlugin (*rp, j.lane->buffer, j.lane->midi, j.n);
        }
    };

    struct StageJob
    {
        RackProcessor* rack;
        RenderStage* stage;
        juce::AudioBuffer<float>* audio;
        juce::MidiBuffer* midi;
        int n;

        static void run (void* p)
        {
            auto& j = *static_cast<StageJob*> (p);
            if (j.n > 0)
                j.rack->processStage (*j.stage, *j.audio, *j.midi, j.n);
        }
    };
}

//==============================================================================
RackProcessor::RackProcessor (GainMeterProcessor& in, GainMeterProcessor& out)
    : AudioProcessor (BusesProperties().withInput  ("In",  juce::AudioChannelSet::stereo(), true)
                                       .withOutput ("Out", juce::AudioChannelSet::stereo(), true)),
      inputStage (in), outputStage (out), pool (defaultWorkerCount())
{
    io.setSize (2, 4096);
}

RackProcessor::~RackProcessor() = default;

bool RackProcessor::isBusesLayoutSupported (const BusesLayout& layout) const
{
    auto ins = layout.getMainInputChannels(), outs = layout.getMainOutputChannels();
    return ins <= 2 && outs >= 1 && outs <= 2;
}

std::unique_ptr<RenderPlan> RackProcessor::swapPlan (std::unique_ptr<RenderPlan> newPlan)
{
    const juce::ScopedLock sl (planLock);
    std::swap (plan, newPlan);
    return newPlan;
}

void RackProcessor::preparePlugin (juce::AudioPluginInstance& inst) const
{
    if (transport != nullptr) inst.setPlayHead (transport);
    inst.setRateAndBufferSizeDetails (currentRate.load(), currentBlock.load());
    inst.prepareToPlay (currentRate.load(), currentBlock.load());
}

void RackProcessor::prepareToPlay (double sampleRate, int newBlockSize)
{
    const bool changed = std::abs (sampleRate - currentRate.load()) > 0.5 || newBlockSize > currentBlock.load();
    currentRate = sampleRate;
    currentBlock = juce::jmax (newBlockSize, 32);

    inputStage.setRateAndBufferSizeDetails (sampleRate, newBlockSize);
    inputStage.prepareToPlay (sampleRate, newBlockSize);
    outputStage.setRateAndBufferSizeDetails (sampleRate, newBlockSize);
    outputStage.prepareToPlay (sampleRate, newBlockSize);

    bool planTooSmall = true;
    {
        const juce::ScopedLock sl (planLock);
        io.setSize (2, juce::jmax (newBlockSize, 32), false, true, true);
        planTooSmall = plan == nullptr || plan->maxBlock < newBlockSize;

        if (plan != nullptr)
        {
            for (auto& item : plan->items)
            {
                if (item->plugin) preparePlugin (*item->plugin->inst);
                for (auto& lane : item->lanes)
                    for (auto& rp : lane.plugins)
                        preparePlugin (*rp->inst);
            }
        }
    }

    // Buffers inside the plan are sized for the old settings: ask for a fresh plan.
    if (changed || planTooSmall)
        if (onNeedsNewPlan)
            juce::MessageManager::callAsync ([cb = onNeedsNewPlan] { cb(); });
}

//==============================================================================
void RackProcessor::processPlugin (RenderPlugin& rp, juce::AudioBuffer<float>& stereo,
                                   const juce::MidiBuffer& midiIn, int n) noexcept
{
    const auto start = juce::Time::getHighResolutionTicks();
    auto* inst = rp.inst;

    // Non-allocating view of the plugin's scratch channels at this block length.
    float* chans[32];
    const int numCh = juce::jmin (rp.numScratch, 32);
    for (int c = 0; c < numCh; ++c)
        chans[c] = rp.scratch.getWritePointer (c);
    juce::AudioBuffer<float> buf (chans, numCh, n);

    auto* l = stereo.getWritePointer (0);
    auto* r = stereo.getWritePointer (1);

    buf.clear();
    if (rp.numIn >= 2)
    {
        juce::FloatVectorOperations::copy (chans[0], l, n);
        juce::FloatVectorOperations::copy (chans[1], r, n);
    }
    else if (rp.numIn == 1)
    {
        juce::FloatVectorOperations::copy (chans[0], l, n);
        juce::FloatVectorOperations::add (chans[0], r, n);
        juce::FloatVectorOperations::multiply (chans[0], 0.5f, n);
    }

    rp.midi.clear();
    if (rp.wantsMidi)
        rp.midi.addEvents (midiIn, 0, n, 0);

    {
        const juce::ScopedLock sl (inst->getCallbackLock());
        if (inst->isSuspended())
            buf.clear();
        else if (rp.slot->bypassed.load (std::memory_order_relaxed))
            inst->processBlockBypassed (buf, rp.midi);
        else
            inst->processBlock (buf, rp.midi);
    }

    if (rp.numOut >= 2)
    {
        juce::FloatVectorOperations::copy (l, chans[0], n);
        juce::FloatVectorOperations::copy (r, chans[1], n);
    }
    else if (rp.numOut == 1)
    {
        juce::FloatVectorOperations::copy (l, chans[0], n);
        juce::FloatVectorOperations::copy (r, chans[0], n);
    }
    // numOut == 0: a MIDI-only plugin, audio passes around it.

    // CPU share of this block, smoothed.
    const double secs = juce::Time::highResolutionTicksToSeconds (juce::Time::getHighResolutionTicks() - start);
    const double rate = currentRate.load (std::memory_order_relaxed);
    const float load = (float) (secs / ((double) n / (rate > 0 ? rate : 48000.0)));
    auto& stored = rp.slot->cpuLoad;
    stored.store (stored.load (std::memory_order_relaxed) * 0.95f + load * 0.05f, std::memory_order_relaxed);
}

void RackProcessor::processItem (RenderItem& item, juce::AudioBuffer<float>& stereo, juce::MidiBuffer& midi, int n) noexcept
{
    if (! item.isSplit)
    {
        processPlugin (*item.plugin, stereo, midi, n);
        return;
    }

    // Both lanes start from the same input.
    for (auto& lane : item.lanes)
    {
        for (int ch = 0; ch < 2; ++ch)
            lane.buffer.copyFrom (ch, 0, stereo, ch, 0, n);
        lane.midi.clear();
        lane.midi.addEvents (midi, 0, n, 0);
    }

    LaneJob jobs[2] = { { this, &item.lanes[0], n }, { this, &item.lanes[1], n } };
    const bool bothBusy = ! item.lanes[0].plugins.empty() && ! item.lanes[1].plugins.empty();

    if (bothBusy && pool.getNumThreads() > 0)
    {
        WorkerPool::Task tasks[2] = { { &LaneJob::run, &jobs[0] }, { &LaneJob::run, &jobs[1] } };
        pool.runParallel (tasks, 2);
    }
    else
    {
        LaneJob::run (&jobs[0]);
        LaneJob::run (&jobs[1]);
    }

    // Line the lanes up when their plugins report different latencies.
    for (auto& lane : item.lanes)
        lane.align.process (lane.buffer, n);

    item.mix.setTargetValue (item.split->mix.load (std::memory_order_relaxed));
    auto* aL = item.lanes[0].buffer.getReadPointer (0);
    auto* aR = item.lanes[0].buffer.getReadPointer (1);
    auto* bL = item.lanes[1].buffer.getReadPointer (0);
    auto* bR = item.lanes[1].buffer.getReadPointer (1);
    auto* oL = stereo.getWritePointer (0);
    auto* oR = stereo.getWritePointer (1);

    for (int i = 0; i < n; ++i)
    {
        const float m = item.mix.getNextValue();
        // Centre = both lanes at unity; the ends fade the other lane out.
        const float ga = juce::jlimit (0.0f, 1.0f, 2.0f * (1.0f - m));
        const float gb = juce::jlimit (0.0f, 1.0f, 2.0f * m);
        oL[i] = aL[i] * ga + bL[i] * gb;
        oR[i] = aR[i] * ga + bR[i] * gb;
    }
}

void RackProcessor::processStage (RenderStage& stage, juce::AudioBuffer<float>& stereo, juce::MidiBuffer& midi, int n) noexcept
{
    for (auto* item : stage.items)
        processItem (*item, stereo, midi, n);
}

void RackProcessor::processPlan (RenderPlan& p, juce::AudioBuffer<float>& stereo, juce::MidiBuffer& midi, int n) noexcept
{
    const int numStages = (int) p.stages.size();

    if (numStages <= 1)
    {
        if (numStages == 1)
            processStage (p.stages[0], stereo, midi, n);
        return;
    }

    // Pipeline: stage k works on what stage k-1 produced one block ago, all stages at once.
    // Rotate so each buffer moves one stage along; the last stage's old buffer becomes the new input slot.
    const int last = p.ring[(size_t) numStages - 1];
    for (int k = numStages - 1; k > 0; --k)
        p.ring[(size_t) k] = p.ring[(size_t) k - 1];
    p.ring[0] = last;

    auto& inBuf = p.stageAudio[(size_t) p.ring[0]];
    for (int ch = 0; ch < 2; ++ch)
        inBuf.copyFrom (ch, 0, stereo, ch, 0, n);
    p.stageMidi[(size_t) p.ring[0]].clear();
    p.stageMidi[(size_t) p.ring[0]].addEvents (midi, 0, n, 0);
    p.stageCount[(size_t) p.ring[0]] = n;

    StageJob jobs[8];
    WorkerPool::Task tasks[8];
    for (int k = 0; k < numStages; ++k)
    {
        const int b = p.ring[(size_t) k];
        const int count = p.stageCount[(size_t) b];
        jobs[k] = { this, &p.stages[(size_t) k], &p.stageAudio[(size_t) b], &p.stageMidi[(size_t) b], count == n ? n : 0 };
        if (count != n) p.stageAudio[(size_t) b].clear();      // block size changed: drop that block
        // Run the last stage on this thread (task 0) so its result is ready first.
        tasks[numStages - 1 - k] = { &StageJob::run, &jobs[k] };
    }

    pool.runParallel (tasks, numStages);

    auto& outBuf = p.stageAudio[(size_t) p.ring[(size_t) numStages - 1]];
    for (int ch = 0; ch < 2; ++ch)
        stereo.copyFrom (ch, 0, outBuf, ch, 0, n);
}

void RackProcessor::processBlock (juce::AudioBuffer<float>& buffer, juce::MidiBuffer& midi)
{
    juce::ScopedNoDenormals noDenormals;
    const int n = buffer.getNumSamples();
    const int numIns = getTotalNumInputChannels();
    const int numOuts = getTotalNumOutputChannels();

    if (n > io.getNumSamples())
    {
        buffer.clear();
        return;
    }

    // Work in a private stereo buffer whatever the device layout is.
    float* chans[2] = { io.getWritePointer (0), io.getWritePointer (1) };
    juce::AudioBuffer<float> stereo (chans, 2, n);

    if (numIns >= 2)       { stereo.copyFrom (0, 0, buffer, 0, 0, n); stereo.copyFrom (1, 0, buffer, 1, 0, n); }
    else if (numIns == 1)  { stereo.copyFrom (0, 0, buffer, 0, 0, n); stereo.copyFrom (1, 0, buffer, 0, 0, n); }
    else                   stereo.clear();

    if (transport != nullptr) transport->beginBlock (getSampleRate());
    inputStage.processBlock (stereo, midi);

    {
        const juce::ScopedLock sl (planLock);
        if (plan != nullptr && plan->maxBlock >= n)
            processPlan (*plan, stereo, midi, n);
    }

    if (transport != nullptr) transport->endBlock (stereo, n);     // the click goes through the master out (and MUTE)
    outputStage.processBlock (stereo, midi);

    for (int ch = 0; ch < buffer.getNumChannels(); ++ch)
    {
        if (ch < numOuts)
            buffer.copyFrom (ch, 0, stereo, juce::jmin (ch, 1), 0, n);
        else
            buffer.clear (ch, 0, n);
    }
}
