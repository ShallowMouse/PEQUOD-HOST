#pragma once

#include <JuceHeader.h>

//==============================================================================
/** The PEQUOD category colours (see "PEQUOD Colour Scheme.png" in the plugin repo).
    PEQUOD plugins are recognised by name; anything else by the category the plugin
    reports about itself. Nothing in the plugins is read or changed. */
namespace PluginColours
{
    struct Category { const char* name; juce::uint32 rgb; const char* plugins; };

    inline const std::vector<Category>& categories()
    {
        static const std::vector<Category> list
        {
            { "Amps & preamps", 0x8E2A22, "AC100|AC100 Bass|AC30 FWN|AC30 Normal|AC30 Top Boost|AC50|Bassman 50|Bassman 6G6-B|Blonde Bassman|Deacy|"
                                          "Deluxe Reverb|DR103 Bass|DR103 Custom 100|Hot Rod Deluxe|Hot Rod DeVille|IVP Preamp|Jazz Chorus|JCM900|JTM45|"
                                          "Rectified|Selmer TB50|Showman AB763|Silverface Twin|SVT Bass|T60 Bass|Traynor YBA-1|Twin Reverb|UL730|"
                                          "Valve Jr Celestion|Valve Jr V30" },
            { "Drive, fuzz & boost", 0xC2611B, "Big Muff|Blues Driver|Centaur|Distortion+|DS-1|EP Booster|First Fuzz|Fuzz Face|Fuzz Factory|"
                                               "Harmonic Percolator|Jekyll & Hyde|Low Drive|Micro Amp|Muff R22|Octavia|OS-2|Power Boost|Rangemaster|"
                                               "RAT|Tone Bender|Treble Booster|TS-808" },
            { "Compressors & limiters", 0xA87A1C, "Barker Bus|Black Box Limiter|Channel Dynamics|Dyna Comp|FET Limiter|Grit Comp|Hard to Explain|"
                                                  "Level Loc|Listen Mic|Opto Comp|Room Crush|RS124|Vari-Mu" },
            { "Expanders, gates & levellers", 0x66661C, "Evenhand|Expander|Slow Gear|Smart Gate" },
            { "Tape & saturation", 0x6B4A2E, "A800 Bus|Ampex 350|Analog Tape|Backwards|Multitrack Tape|Repro Head|Studer 169|Tape Loop|"
                                             "Tape Machine|Twin-Track Tape|Varispeed|Warmer" },
            { "Consoles, preamps & strips", 0x46596B, "Ampex 351|Gold Star Desk|Neotek Channel|Plank Console|TG Channel|TG Strip|V72 V76|"
                                                      "Valve Desk|Valve Mixer|Vocal Chain" },
            { "EQ & filters", 0x2C4A9A, "Black Box EQ|Envelope Filter|GE-7 EQ|Studio EQ|Talk Box|Wah" },
            { "Delays & echoes", 0x1E5A84, "AMS 15-80S|Brighton Rock Delay|Chorus Echo|Copicat|Digital Delay|DL4|DM-1 Delay|Doubler|Echorec|"
                                           "Ford Space Station|Memory Man|Ping-Pong Delay|Plank Echo|Quad Tracker|RE-301|Revox Loop|Rhythm Taps|"
                                           "Space Station|Tape Echo|Three-Head Echo" },
            { "Plates, springs & reverbs", 0x1F5F63, "Cloud|EMT Plate|Hall|Heater Spring|Nonlin|Plate 140|Reverse Reverb|Shimmer|Soaked|"
                                                     "Spring Reverb|Strawberry Plate" },
            { "Rooms, chambers & spaces", 0x1F5E4A, "Albini Rooms|Antonoff Rooms|blink Rooms|Deltalab Rooms|Echo Chamber|Floyd Spaces|Ford Rooms|"
                                                    "Gold Star Chamber|Hannett Rooms|Meek Rooms|Monkeys Rooms|Plank Rooms|Queen Rooms|sombr Rooms|"
                                                    "Spector Rooms|Stray Cats Rooms|Strokes Rooms|Studio Spaces|Townhouse Rooms" },
            { "Modulation", 0x5C2E7A, "Auto-Pan|Bi-Phase|Bigsby|CE-2 Chorus|Deja Vibe|Dimension|Electric Mistress|Flanger|Hand Phaser|"
                                      "Harmonic Trem|Pad Chopper|Phase 90|Ring Mod|Rotary Speakers|Rotor|Slicer|Small Clone|Small Stone|"
                                      "Tape Flanger|Time Modulator|TR-2 Tremolo|Uni-Vibe|Vibrato|Wall of Sound" },
            { "Synths, keys & instruments", 0x3F6B2A, "6120 Voice|Bass Voice|Clavioline|Compact Organ|Electric Grand|Juno Six|P-Bass Voice|"
                                                      "Red Special|Synare 3|Tonewheel Organ|TR-606|Twenty-Six|Upright Slap|VCS3" },
            { "Lo-fi & character", 0x8C2F62, "Car Radio|Cheap Speaker|Lo-Fi Crush|Phone Line|Practice Amp Vocal" },
            { "Microphones", 0x444B52, "Mic Pair|Studio Mics" },
            { "Experimental & pitch", 0x3B336B, "Drop Tune|Feedback|Freeze|Guitar Orchestra|Harmonizer|HOG|POG|Sympathy|Whammy" },
            { "Utilities", 0x2B2F33, "Looper|Tuner" },
        };
        return list;
    }

    inline juce::String normalise (const juce::String& s)
    {
        return s.toLowerCase().retainCharacters ("abcdefghijklmnopqrstuvwxyz0123456789");
    }

    /** Index into categories(), or -1. */
    inline int categoryOf (const juce::PluginDescription& d)
    {
        static const std::map<juce::String, int> byName = []
        {
            std::map<juce::String, int> m;
            auto& cats = categories();
            for (int i = 0; i < (int) cats.size(); ++i)
                for (auto& n : juce::StringArray::fromTokens (cats[(size_t) i].plugins, "|", {}))
                    m[normalise (n)] = i;
            return m;
        }();

        if (auto it = byName.find (normalise (d.name)); it != byName.end()) return it->second;

        // Not one we know: go by what the plugin says it is (VST3 sub-categories such as "Fx|Delay").
        auto c = (d.category + " " + d.name).toLowerCase();
        auto has = [&] (std::initializer_list<const char*> words)
        {
            for (auto* w : words) if (c.contains (w)) return true;
            return false;
        };
        if (has ({ "amp", "cab" }))                              return 0;
        if (has ({ "distortion", "drive", "fuzz", "boost" }))    return 1;
        if (has ({ "gate", "expander" }))                        return 3;
        if (has ({ "dynamics", "compress", "limit" }))           return 2;
        if (has ({ "tape", "saturat" }))                         return 4;
        if (has ({ "eq", "filter", "wah" }))                     return 6;
        if (has ({ "delay", "echo" }))                           return 7;
        if (has ({ "room", "chamber", "space" }))                return 9;
        if (has ({ "reverb", "plate", "spring" }))               return 8;
        if (has ({ "modulation", "chorus", "flang", "phase", "trem", "vibrato", "rotary" })) return 10;
        if (has ({ "instrument", "synth", "organ", "piano" }))   return 11;
        if (has ({ "pitch", "harmon", "octave" }))               return 14;
        if (has ({ "tools", "analyzer", "tuner", "loop" }))      return 15;
        return -1;
    }

    inline juce::Colour forPlugin (const juce::PluginDescription& d)
    {
        const int i = categoryOf (d);
        return i < 0 ? juce::Colour (0xffe0a82e) : juce::Colour (0xff000000 | categories()[(size_t) i].rgb);
    }

    /** The panel colours are deep; an LED shows the same hue at full strength (otherwise it looks almost off). */
    inline juce::Colour forLed (juce::Colour c)
    {
        const float top = (float) juce::jmax (c.getRed(), c.getGreen(), c.getBlue());
        if (top <= 0.0f) return juce::Colours::white;
        const float k = 255.0f / top;
        return juce::Colour ((juce::uint8) juce::jmin (255.0f, (float) c.getRed() * k),
                             (juce::uint8) juce::jmin (255.0f, (float) c.getGreen() * k),
                             (juce::uint8) juce::jmin (255.0f, (float) c.getBlue() * k));
    }
}
