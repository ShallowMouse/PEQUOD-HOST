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
        case Type::macro:     return "MACRO KNOB";
        case Type::pitch:     return "PITCH FOLLOWER";
        case Type::random:    return "RANDOMISER";
        case Type::ducker:    return "DUCKER";
        case Type::swell:     return "SWELL";
        case Type::counter:   return "NOTE STEPPER";
    }
    return "MODIFIER";
}

const char* curveName (Curve c)
{
    switch (c)
    {
        case Curve::linear:    return "STRAIGHT";
        case Curve::slowStart: return "SLOW START";
        case Curve::fastStart: return "FAST START";
        case Curve::sCurve:    return "S-CURVE";
        case Curve::steps4:    return "4 STEPS";
        case Curve::steps8:    return "8 STEPS";
    }
    return "";
}

float applyCurve (int curve, float v)
{
    v = juce::jlimit (0.0f, 1.0f, v);
    switch ((Curve) curve)
    {
        case Curve::linear:    return v;
        case Curve::slowStart: return v * v;
        case Curve::fastStart: return std::sqrt (v);
        case Curve::sCurve:    return v * v * (3.0f - 2.0f * v);
        case Curve::steps4:    return juce::jmin (1.0f, std::floor (v * 4.0f) / 3.0f);
        case Curve::steps8:    return juce::jmin (1.0f, std::floor (v * 8.0f) / 7.0f);
    }
    return v;
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
    macroValue = 0.0f; lowNote = 40.0f; highNote = 88.0f; glideMs = 80.0f; randomMode = randomDrift;
    thresholdDb = -45.0f; holdMs = 150.0f;
    rateMod = -1.0f; amountMod = -1.0f; macroMod = -1.0f;
    randValue = randTarget = 0.0f; duckLevel = 0.0f; swellLevel = 0.0f; swellUp = false; counterStep = 0; counterValue = 0.0f; pitchValue = 0.0f;
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

    // A pick attack: the fast level jumps well above the slow one, above the threshold.
    bool detectPick (Slot& s, const BlockInfo& b, double dt)
    {
        s.fastDb += (b.inputLevelDb - s.fastDb) * blockCoeff (1.0, dt);
        s.slowDb += (b.inputLevelDb - s.slowDb) * blockCoeff (120.0, dt);
        s.pickHoldoff -= dt;
        // One trigger per pick: after firing, wait until the attack has passed (fast back near slow) to re-arm.
        if (! s.pickArmed && s.fastDb - s.slowDb < 2.0f && s.pickHoldoff <= 0.0) s.pickArmed = true;
        if (s.pickArmed && s.fastDb > s.pickThresholdDb.load() && s.fastDb - s.slowDb > 6.0f)
        {
            s.pickArmed = false;
            s.pickHoldoff = 0.05;
            return true;
        }
        return false;
    }

    bool switchPressed (Slot& s)
    {
        const int presses = s.pressCount.load();
        if (presses == s.seenPresses) return false;
        s.seenPresses = presses;
        return true;
    }

    int effectiveDivision (const Slot& s)
    {
        const float m = s.rateMod.load();
        if (m < 0.0f) return s.division.load();
        return juce::roundToInt (juce::jlimit (0.0f, 1.0f, m) * (float) ((int) divisions().size() - 1));
    }
    double effectiveRateHz (const Slot& s)
    {
        const float m = s.rateMod.load();
        return m < 0.0f ? (double) s.rateHz.load() : rateFromNorm (m);
    }
}

bool usesDivision (const Slot& s)
{
    const auto t = (Type) s.type.load();
    return (t == Type::lfo && s.sync.load()) || t == Type::sequencer
        || (t == Type::random && s.randomMode.load() == Slot::randomBeat);
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
                const double len = divisionBeats (effectiveDivision (s), b.beatsPerBar);
                ph = b.ppq / len + s.phase.load();
            }
            else
            {
                s.lfoPhase += effectiveRateHz (s) * dt;
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
                    if (switchPressed (s)) start = true;
                    if (s.holdMode && ! s.held.load() && s.envStage > 0 && s.envStage < 4) release = true;
                    break;
                }
                case Slot::triggerPicking:
                {
                    if (detectPick (s, b, dt)) start = true;
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
            const double len = divisionBeats (effectiveDivision (s), b.beatsPerBar);
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

        case Type::macro:
        {
            const float m = s.macroMod.load();
            v = m >= 0.0f ? m : s.macroValue.load();
            break;
        }

        case Type::pitch:
        {
            // Holds the last note between notes.
            if (b.pitchHz > 20.0f)
            {
                const float note = 69.0f + 12.0f * std::log2 (b.pitchHz / 440.0f);
                const float lo = s.lowNote.load(), hi = juce::jmax (lo + 1.0f, s.highNote.load());
                const float target = juce::jlimit (0.0f, 1.0f, (note - lo) / (hi - lo));
                s.pitchValue += (target - s.pitchValue) * blockCoeff (s.glideMs.load(), dt);
            }
            v = s.pitchValue;
            break;
        }

        case Type::random:
        {
            bool next = false;
            switch (s.randomMode.load())
            {
                case Slot::randomPick:   next = detectPick (s, b, dt); break;
                case Slot::randomSwitch: next = switchPressed (s); break;
                case Slot::randomBeat:
                {
                    const auto beat = (int64_t) std::floor (b.ppq / divisionBeats (effectiveDivision (s), b.beatsPerBar));
                    next = beat != s.lastBeat;
                    s.lastBeat = beat;
                    break;
                }
                default:
                {
                    // drift: a new target at the rate, glided to
                    s.lfoPhase += effectiveRateHz (s) * dt;
                    if (s.lfoPhase >= 1.0) { s.lfoPhase -= std::floor (s.lfoPhase); next = true; }
                    break;
                }
            }
            if (next) { s.randTarget = s.rng.nextFloat(); s.fired = s.randomMode.load() != Slot::randomDrift; }
            const double glide = s.randomMode.load() == Slot::randomDrift ? 1000.0 / juce::jmax (0.05, effectiveRateHz (s)) * 0.5
                                                                          : (double) s.glideMs.load();
            s.randValue += (s.randTarget - s.randValue) * blockCoeff (glide, dt);
            v = s.randValue;
            break;
        }

        case Type::ducker:
        {
            // 1 while you play (after a quick attack), back to 0 after the hold and release once you stop.
            const bool playing = b.inputLevelDb > s.thresholdDb.load();
            if (playing) s.duckHoldLeft = s.holdMs.load() * 0.001;
            else         s.duckHoldLeft -= dt;
            const bool ducked = playing || s.duckHoldLeft > 0.0;
            const float target = ducked ? 1.0f : 0.0f;
            s.duckLevel += (target - s.duckLevel) * blockCoeff (ducked ? s.attackMs.load() : s.releaseMs.load(), dt);
            v = s.duckLevel;
            break;
        }

        case Type::swell:
        {
            // HOLD: up while the switch is held, down when it's let go. Otherwise each press flips up / down.
            const bool pressed = switchPressed (s);
            if (s.holdMode)   s.swellUp = s.held.load();
            else if (pressed) s.swellUp = ! s.swellUp;
            const float t = s.swellUp ? 1.0f : 0.0f;
            const double ms = s.swellUp ? s.aMs.load() : s.rMs.load();
            const float step = (float) (dt / juce::jmax (0.001, ms * 0.001));
            s.swellLevel = s.swellUp ? juce::jmin (t, s.swellLevel + step) : juce::jmax (t, s.swellLevel - step);
            v = s.swellLevel;
            break;
        }

        case Type::counter:
        {
            // A step sequencer that moves on with each note you pick (or each press / MIDI note).
            bool advance = false;
            switch (s.trigger.load())
            {
                case Slot::triggerPicking:   advance = detectPick (s, b, dt); break;
                case Slot::triggerMidiNotes: advance = b.noteOns > 0; break;
                default:                     advance = switchPressed (s); break;
            }
            const int count = juce::jlimit (1, maxSteps, s.length.load());
            if (advance) { s.counterStep = (s.counterStep + 1) % count; s.fired = true; }
            if (s.counterStep >= count) s.counterStep = 0;
            const float target = s.steps[s.counterStep].load();
            const float g = s.glide.load();
            s.counterValue = g > 0.001f ? s.counterValue + (target - s.counterValue) * blockCoeff (g * 300.0, dt) : target;
            s.currentStep = s.counterStep;
            v = s.counterValue;
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
