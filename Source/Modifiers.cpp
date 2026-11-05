#include "Modifiers.h"

namespace Modifiers
{
const char* typeName (Type t)
{
    switch (t)
    {
        case Type::lfo:       return "LFO";
        case Type::follower:  return "ENVELOPE FOLLOWER";
        case Type::envelope:  return "TRIGGERED ENVELOPE";
        case Type::sequencer: return "STEP SEQUENCER";
    }
    return "MODIFIER";
}

const char* shapeName (Shape s)
{
    switch (s)
    {
        case Shape::sine:         return "SINE";
        case Shape::triangle:     return "TRIANGLE";
        case Shape::square:       return "SQUARE";
        case Shape::sawUp:        return "RAMP UP";
        case Shape::sawDown:      return "RAMP DOWN";
        case Shape::sampleHold:   return "RANDOM STEPS";
        case Shape::smoothRandom: return "RANDOM SMOOTH";
    }
    return "";
}

const std::vector<Division>& divisions()
{
    // Negative = bars (follows the metronome's beats per bar).
    static const std::vector<Division> list {
        { -4.0, "4 BARS" }, { -2.0, "2 BARS" }, { -1.0, "1 BAR" },
        { 2.0, "1/2" }, { 1.0, "1/4" }, { 2.0 / 3.0, "1/4 T" },
        { 0.75, "1/8 ." }, { 0.5, "1/8" }, { 1.0 / 3.0, "1/8 T" },
        { 0.375, "1/16 ." }, { 0.25, "1/16" }, { 1.0 / 6.0, "1/16 T" }, { 0.125, "1/32" },
    };
    return list;
}

int defaultDivision() { return 4; }

void Slot::resetSettings()
{
    on = true; amount = 1.0f; invert = false;
    shape = 0; sync = true; division = defaultDivision(); rateHz = 1.0f; phase = 0.0f;
    sensitivity = 20.0f; attackMs = 10.0f; releaseMs = 150.0f;
    trigger = triggerSwitch; holdMode = false; aMs = 5.0f; dMs = 300.0f; sustain = 0.0f; rMs = 400.0f; pickThresholdDb = -40.0f;
    for (int i = 0; i < maxSteps; ++i) steps[i] = (i % 2 == 0) ? 1.0f : 0.25f;
    length = 8; glide = 0.0f;
    lfoPhase = 0.0; followDb = -100.0f; envLevel = 0.0f; envStage = 0; lastOut = 0.0f;
}

namespace
{
    double divisionBeats (int index, int beatsPerBar)
    {
        const auto& d = divisions();
        const auto beats = d[(size_t) juce::jlimit (0, (int) d.size() - 1, index)].beats;
        return beats < 0 ? -beats * juce::jmax (1, beatsPerBar) : beats;
    }

    float shapeValue (Slot& s, double ph, bool wrapped)
    {
        ph -= std::floor (ph);
        switch ((Shape) s.shape.load())
        {
            case Shape::sine:     return 0.5f + 0.5f * (float) std::sin (juce::MathConstants<double>::twoPi * ph - juce::MathConstants<double>::halfPi);
            case Shape::triangle: return (float) (ph < 0.5 ? ph * 2.0 : 2.0 - ph * 2.0);
            case Shape::square:   return ph < 0.5 ? 1.0f : 0.0f;
            case Shape::sawUp:    return (float) ph;
            case Shape::sawDown:  return (float) (1.0 - ph);
            case Shape::sampleHold:
                if (wrapped) s.randB = s.rng.nextFloat();
                return s.randB;
            case Shape::smoothRandom:
                if (wrapped) { s.randA = s.randB; s.randB = s.rng.nextFloat(); }
                return s.randA + (s.randB - s.randA) * (float) (0.5 - 0.5 * std::cos (juce::MathConstants<double>::pi * ph));
        }
        return 0.0f;
    }

    // One-pole smoothing over a block: how far to move towards the target this block.
    float blockCoeff (double ms, double dt)
    {
        return ms <= 0.0 ? 1.0f : (float) (1.0 - std::exp (-dt / (ms * 0.001)));
    }
}

float process (Slot& s, const BlockInfo& b)
{
    const double dt = (double) b.numSamples / juce::jmax (1.0, b.sampleRate);
    float v = 0.0f;

    switch ((Type) s.type.load())
    {
        case Type::lfo:
        {
            double ph;
            if (s.sync)
            {
                const double len = divisionBeats (s.division, b.beatsPerBar);
                ph = b.ppq / len + s.phase.load();
            }
            else
            {
                s.lfoPhase += s.rateHz.load() * dt;
                ph = s.lfoPhase + s.phase.load();
            }
            const double whole = std::floor (ph);
            const bool wrapped = whole != s.lastWhole;     // a new cycle: random shapes pick a new value
            s.lastWhole = whole;
            v = shapeValue (s, ph, wrapped);
            break;
        }

        case Type::follower:
        {
            // Level in dB, sensitivity shifts it into a 60 dB window: quiet = 0, loud = 1.
            const float target = b.inputLevelDb;
            const float c = blockCoeff (target > s.followDb ? s.attackMs.load() : s.releaseMs.load(), dt);
            s.followDb += (target - s.followDb) * c;
            v = juce::jlimit (0.0f, 1.0f, (s.followDb + 60.0f - (20.0f - s.sensitivity.load())) / 60.0f);
            break;
        }

        case Type::envelope:
        {
            bool start = false, release = false;
            switch (s.trigger.load())
            {
                case Slot::triggerSwitch:
                {
                    const int presses = s.pressCount.load();
                    if (presses != s.seenPresses) { s.seenPresses = presses; start = true; }
                    if (s.holdMode && ! s.held.load() && s.envStage > 0 && s.envStage < 4) release = true;
                    break;
                }
                case Slot::triggerPicking:
                {
                    // A pick attack: the fast level jumps well above the slow one, above the threshold.
                    s.fastDb += (b.inputLevelDb - s.fastDb) * blockCoeff (1.0, dt);
                    s.slowDb += (b.inputLevelDb - s.slowDb) * blockCoeff (120.0, dt);
                    s.pickHoldoff -= dt;
                    if (s.fastDb > s.pickThresholdDb.load() && s.fastDb - s.slowDb > 6.0f && s.pickHoldoff <= 0.0)
                    {
                        start = true;
                        s.pickHoldoff = 0.08;
                    }
                    if (s.holdMode && s.fastDb < s.pickThresholdDb.load() - 6.0f && s.envStage > 0 && s.envStage < 4) release = true;
                    break;
                }
                case Slot::triggerMidiNotes:
                    if (b.noteOns > 0) start = true;
                    if (s.holdMode && ! b.notesHeld && s.envStage > 0 && s.envStage < 4) release = true;
                    break;
                default: break;
            }

            if (start) { s.envStage = 1; s.fired = true; }
            else if (release) s.envStage = 4;

            const float sus = s.sustain.load();
            switch (s.envStage)
            {
                case 1:  // attack
                    s.envLevel += (float) (dt / juce::jmax (0.001, s.aMs.load() * 0.001));
                    if (s.envLevel >= 1.0f) { s.envLevel = 1.0f; s.envStage = 2; }
                    break;
                case 2:  // decay to sustain
                    s.envLevel -= (float) (dt / juce::jmax (0.001, s.dMs.load() * 0.001)) * (1.0f - sus);
                    if (s.envLevel <= sus) { s.envLevel = sus; s.envStage = s.holdMode ? 3 : 4; }
                    break;
                case 3:  // sustain (hold mode)
                    s.envLevel = sus;
                    break;
                case 4:  // release
                    s.envLevel -= (float) (dt / juce::jmax (0.001, s.rMs.load() * 0.001));
                    if (s.envLevel <= 0.0f) { s.envLevel = 0.0f; s.envStage = 0; }
                    break;
                default: s.envLevel = 0.0f; break;
            }
            v = s.envLevel;
            break;
        }

        case Type::sequencer:
        {
            const double len = divisionBeats (s.division, b.beatsPerBar);
            const double pos = b.ppq / len;
            const int count = juce::jlimit (1, maxSteps, s.length.load());
            const auto whole = (int64_t) std::floor (pos);
            const int step = (int) (((whole % count) + count) % count);
            const double frac = pos - std::floor (pos);
            const float here = s.steps[step].load(), next = s.steps[(step + 1) % count].load();
            const float g = s.glide.load();
            v = here;
            if (g > 0.001f && frac > 1.0 - g)
                v = here + (next - here) * (float) ((frac - (1.0 - g)) / g);
            s.currentStep = step;
            break;
        }

        default: break;
    }

    if (s.invert) v = 1.0f - v;
    v = juce::jlimit (0.0f, 1.0f, v);
    s.out.store (v, std::memory_order_relaxed);
    s.lastOut = v;
    return v;
}
}
