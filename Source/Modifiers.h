#pragma once

#include <JuceHeader.h>

//==============================================================================
/** Modifiers: LFOs, envelope followers, triggered envelopes and step sequencers that move plugin
    knobs. Each makes a value 0..1 every audio block; each assignment turns that into a knob
    position between its FROM and TO. Saved with the preset. */
namespace Modifiers
{
    enum class Type { lfo = 1, follower = 2, envelope = 3, sequencer = 4 };
    enum class Shape { sine, triangle, square, sawUp, sawDown, sampleHold, smoothRandom };
    constexpr int maxModifiers = 16;
    constexpr int maxSteps = 16;

    const char* typeName (Type);
    const char* shapeName (Shape);
    constexpr int numShapes = 7;

    /** Tempo divisions for synced LFOs and sequencers: length in beats (quarter notes) and a label. */
    struct Division { double beats; const char* label; };
    const std::vector<Division>& divisions();
    int defaultDivision();                       // 1/4

    /** Free LFO rate: 0..1 to 0.05..20 Hz (exponential), and back. */
    inline double rateFromNorm (double v) { return 0.05 * std::pow (400.0, juce::jlimit (0.0, 1.0, v)); }
    inline double normFromRate (double hz) { return std::log (juce::jlimit (0.05, 20.0, hz) / 0.05) / std::log (400.0); }

    //==========================================================================
    /** One modifier's settings (written by the UI, read by the audio thread) and running state (audio thread). */
    struct Slot
    {
        std::atomic<int> type { 0 };             // 0 = free slot
        std::atomic<bool> on { true };
        std::atomic<float> amount { 1.0f };      // scales every assignment's sweep
        std::atomic<bool> invert { false };

        // LFO
        std::atomic<int> shape { 0 };
        std::atomic<bool> sync { true };
        std::atomic<int> division { 4 };         // index into divisions()
        std::atomic<float> rateHz { 1.0f };
        std::atomic<float> phase { 0.0f };       // 0..1 start offset (synced)

        // Envelope follower
        std::atomic<float> sensitivity { 20.0f }; // dB of boost before the 60 dB window
        std::atomic<float> attackMs { 10.0f }, releaseMs { 150.0f };

        // Triggered envelope (ADSR)
        enum Trigger { triggerSwitch = 0, triggerPicking = 1, triggerMidiNotes = 2 };
        std::atomic<int> trigger { triggerSwitch };
        std::atomic<bool> holdMode { false };     // true: sustain while the switch / note is held; false: one shot
        std::atomic<float> aMs { 5.0f }, dMs { 300.0f }, sustain { 0.0f }, rMs { 400.0f };
        std::atomic<float> pickThresholdDb { -40.0f };
        std::atomic<int> pressCount { 0 };       // bumped by footswitch / FIRE button
        std::atomic<bool> held { false };

        // Step sequencer
        std::atomic<float> steps[maxSteps];
        std::atomic<int> length { 8 };
        std::atomic<float> glide { 0.0f };       // 0..1 of a step

        // Output (for the UI)
        std::atomic<float> out { 0.0f };
        std::atomic<int> currentStep { 0 };
        std::atomic<bool> fired { false };       // a trigger happened (UI flashes)

        Slot() { for (int i = 0; i < maxSteps; ++i) steps[i] = (i % 2 == 0) ? 1.0f : 0.25f; }
        void resetSettings();

        //== audio thread
        double lfoPhase = 0.0, lastWhole = 0.0;
        float randA = 0, randB = 0;
        float followDb = -100.0f;
        float envLevel = 0.0f; int envStage = 0;   // 0 idle 1 attack 2 decay 3 sustain 4 release
        int seenPresses = 0;
        float fastDb = -100.0f, slowDb = -100.0f; double pickHoldoff = 0;
        float lastOut = 0.0f;
        juce::Random rng;
    };

    /** One knob a modifier moves: between from and to (normalised 0..1). */
    struct Assignment
    {
        juce::String slotId;       // the plugin's stable id in the preset
        int paramIndex = -1;
        std::atomic<float> from { 0.0f }, to { 1.0f };
    };

    /** What the audio thread knows about the block. */
    struct BlockInfo
    {
        double sampleRate = 48000.0;
        int numSamples = 0;
        double ppq = 0.0;              // where the beat is (the metronome runs all the time)
        int beatsPerBar = 4;
        float inputLevelDb = -100.0f;  // guitar input (before the rack)
        int noteOns = 0;               // MIDI notes in this block
        int noteOffs = 0;
        bool notesHeld = false;
    };

    /** Works out a slot's value for this block (0..1). Audio thread. */
    float process (Slot&, const BlockInfo&);
}
