#pragma once

#include "PluginColours.h"

//==============================================================================
/** Tags for the browser: every plugin's COLLECTION (which PEQUOD collection it comes from,
    or its maker for anything else) and FUNCTION (amp, drive, delay... the PEQUOD colour
    categories), plus the key used to hide duplicate copies of the same plugin. */
namespace PluginTags
{
    /** The separator between a collection's artist and its title (a middle dot). */
    inline juce::String sep() { return " " + juce::String::charToString ((juce::juce_wchar) 0x00b7) + " "; }

    /** PEQUOD vendor names -> the collection's title, as on the zips and in the repo. */
    inline juce::String collectionTitle (const juce::String& vendorSuffix)
    {
        static const std::map<juce::String, std::pair<const char*, const char*>> titles
        {
            { "beatles",                  { "Beatles", "Studio Two" } },
            { "pinkfloyd",                { "Pink Floyd", "Britannia Row" } },
            { "albini",                   { "Albini", "Ironsides" } },
            { "joemeek",                  { "Joe Meek", "Holloway Road" } },
            { "spector",                  { "Spector", "Gold Star" } },
            { "hannett",                  { "Hannett", "Strawberry" } },
            { "plank",                    { "Conny Plank", "Conny's Studio" } },
            { "padghamlillywhite",        { "Padgham & Lillywhite", "Townhouse" } },
            { "jamesford",                { "James Ford", "Repro" } },
            { "sombr",                    { "sombr", "Bedroom to Sound City" } },
            { "antonoff",                 { "Antonoff", "Rough Customer" } },
            { "briansetzer",              { "Brian Setzer", "Built for Speed" } },
            { "brianmay",                 { "Brian May", "Fireplace" } },
            { "arcticmonkeys",            { "Arctic Monkeys", "Rancho" } },
            { "thestrokes",               { "The Strokes", "Transporterraum" } },
            { "blocparty",                { "Bloc Party", "Deltalab" } },
            { "blink182",                 { "blink-182", "Poway" } },
            { "pedals",                   { "PEQUOD Pedals", nullptr } },
        };
        auto it = titles.find (PluginColours::normalise (vendorSuffix));
        if (it == titles.end()) return vendorSuffix;
        auto t = juce::String (it->second.first);
        return it->second.second != nullptr ? t + sep() + it->second.second : t;
    }

    /** "Arctic Monkeys - Rancho", "PEQUOD Pedals", "PEQUOD" (the standalone tools), or the maker. */
    inline juce::String collectionOf (const juce::PluginDescription& d)
    {
        auto maker = d.manufacturerName.trim();
        if (maker.isEmpty()) return "Unknown";
        if (maker.equalsIgnoreCase ("PEQUOD")) return "PEQUOD";
        if (maker.startsWithIgnoreCase ("PEQUOD "))
            return collectionTitle (maker.substring (7).trim());
        return maker;
    }

    inline bool isPequod (const juce::PluginDescription& d)
    {
        return d.manufacturerName.trim().startsWithIgnoreCase ("PEQUOD");
    }

    /** "Amps & preamps", "Delays & echoes"... or "Other". */
    inline juce::String functionOf (const juce::PluginDescription& d)
    {
        if (d.isInstrument) return PluginColours::categories()[11].name;
        const int i = PluginColours::categoryOf (d);
        return i < 0 ? juce::String ("Other") : juce::String (PluginColours::categories()[(size_t) i].name);
    }

    /** Position of a function in the colour-scheme order (amps first ... utilities, then Other). */
    inline int functionOrder (const juce::PluginDescription& d)
    {
        if (d.isInstrument) return 11;
        const int i = PluginColours::categoryOf (d);
        return i < 0 ? 999 : i;
    }

    /** Two entries with the same key are the same plugin installed twice (for example once in its
        collection's folder and once in the PEQUOD Pedals folder). */
    inline juce::String duplicateKey (const juce::PluginDescription& d)
    {
        return PluginColours::normalise (d.name) + "|" + PluginColours::normalise (d.manufacturerName)
             + "|" + juce::String (d.isInstrument ? "i" : "f");
    }
}
