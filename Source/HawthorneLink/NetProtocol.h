#pragma once

#include <juce_core/juce_core.h>
#include <cstdint>
#include <cstring>

namespace hawthorne::net
{

//==============================================================================
/**
    Hawthorne Link: uncompressed multichannel audio between PCs on a LAN.

    Discovery   sender broadcasts  "HWNA1 ANN <id> <ctlPort> <channels> <rate> <session>\n<name>"
                to announcePort every second (and replies the same to "HWNA1 QRY",
                so senders on other subnets / VLANs can be added by IP).
    Subscribe   receiver sends "HWNA1 SUB <id> <receiverId>" to the sender's control port every
                second from its data socket; the sender streams to that address
                until it hasn't heard from it for 5 s ("HWNA1 BYE <id>" ends it at once).
    Audio       UDP datagrams, header + 24-bit little-endian interleaved PCM. Every
                frame carries its exact position in the sender's sample clock, so the
                receiver puts it in exactly the right place and can tell when the two
                machines share a word clock (then nothing is resampled: bit-perfect).
                Redundancy: each packet also carries the previous block, so any single
                lost packet costs nothing.
*/
constexpr int announcePort = 47201;
constexpr int controlPort  = 47202;     // sender (falls back to any free port)
constexpr int dataPort     = 47203;     // receiver (falls back to any free port)
constexpr int maxChannels  = 64;
constexpr int maxDatagram  = 1472;      // fits a standard 1500-byte Ethernet MTU unfragmented
constexpr int subscriptionTimeoutMs = 5000;

constexpr uint32_t magic = 0x414e5748;  // "HWNA" little-endian
constexpr uint8_t  version = 1;

#pragma pack(push, 1)
struct PacketHeader
{
    uint32_t magic;
    uint8_t  version;
    uint8_t  flags;          // bit 0: block B (previous block) present
    uint16_t channels;
    uint32_t senderId;       // random per sender app run
    uint32_t session;        // random per audio-device start: frame numbers restart
    uint32_t sampleRate;
    uint16_t frames;         // frames per block
    uint16_t latency;        // sender's input latency + audio buffer, samples (information)
    uint64_t firstFrame;     // block A
    uint64_t prevFirstFrame; // block B
};
#pragma pack(pop)
static_assert (sizeof (PacketHeader) == 40, "header layout");

constexpr int headerSize = (int) sizeof (PacketHeader);

/** Frames per block that keep a datagram within maxDatagram. */
inline int framesPerPacket (int channels, bool redundant) noexcept
{
    const int bytesPerFrame = juce::jmax (1, channels) * 3 * (redundant ? 2 : 1);
    return juce::jlimit (1, 1024, (maxDatagram - headerSize) / bytesPerFrame);
}

//==============================================================================
// 24-bit PCM. float <-> int24 is exact for anything that came from a 24-bit converter.
inline void writeInt24 (uint8_t* d, float v) noexcept
{
    const double s = juce::jlimit (-8388608.0, 8388607.0, std::round ((double) v * 8388608.0));
    const auto i = (int32_t) s;
    d[0] = (uint8_t) (i & 0xff);
    d[1] = (uint8_t) ((i >> 8) & 0xff);
    d[2] = (uint8_t) ((i >> 16) & 0xff);
}

inline float readInt24 (const uint8_t* d) noexcept
{
    int32_t i = (int32_t) d[0] | ((int32_t) d[1] << 8) | ((int32_t) d[2] << 16);
    if (i & 0x800000) i |= ~0xffffff;
    return (float) i / 8388608.0f;
}

//==============================================================================
// Control messages
struct Announce
{
    uint32_t senderId = 0, session = 0;
    int controlPort = 0, channels = 0;
    double sampleRate = 0.0;
    juce::String name;
};

inline juce::String hex (uint32_t v) { return juce::String::toHexString ((juce::int64) v).paddedLeft ('0', 8); }
inline uint32_t fromHex (const juce::String& s) { return (uint32_t) s.getHexValue64(); }

inline juce::String makeAnnounce (const Announce& a)
{
    return "HWNA1 ANN " + hex (a.senderId) + " " + juce::String (a.controlPort) + " " + juce::String (a.channels) + " "
         + juce::String ((int) a.sampleRate) + " " + hex (a.session) + "\n" + a.name;
}

inline bool parseAnnounce (const juce::String& msg, Announce& a)
{
    if (! msg.startsWith ("HWNA1 ANN "))
        return false;
    auto first = msg.upToFirstOccurrenceOf ("\n", false, false);
    auto t = juce::StringArray::fromTokens (first, " ", {});
    if (t.size() < 7) return false;
    a.senderId    = fromHex (t[2]);
    a.controlPort = t[3].getIntValue();
    a.channels    = juce::jlimit (0, maxChannels, t[4].getIntValue());
    a.sampleRate  = t[5].getDoubleValue();
    a.session     = fromHex (t[6]);
    a.name        = msg.fromFirstOccurrenceOf ("\n", false, false).trim();
    return a.senderId != 0 && a.controlPort > 0;
}

/** Bigger kernel socket buffers than the OS default (Windows' is small). In NetSender.cpp. */
void setSocketBuffers (int socketHandle, int receiveBytes, int sendBytes);

inline juce::String makeSubscribe (uint32_t senderId, uint32_t receiverId) { return "HWNA1 SUB " + hex (senderId) + " " + hex (receiverId); }
inline juce::String makeBye (uint32_t senderId)         { return "HWNA1 BYE " + hex (senderId); }
inline juce::String makeQuery()                         { return "HWNA1 QRY"; }

} // namespace hawthorne::net
