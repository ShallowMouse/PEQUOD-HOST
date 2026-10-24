#include "NetSender.h"

namespace hawthorne
{

//==============================================================================
void Packetizer::prepare (uint32_t id, uint32_t sess, double sr, int ch, bool red)
{
    channels = juce::jlimit (1, net::maxChannels, ch);
    redundant = red;
    fpp = net::framesPerPacket (channels, redundant);
    filled = 0;
    nextFrame = 0;
    havePrev = false;

    const size_t blockBytes = (size_t) fpp * (size_t) channels * 3;
    blockA.assign (blockBytes, 0);
    blockB.assign (blockBytes, 0);
    datagram.assign ((size_t) net::headerSize + 2 * blockBytes, 0);

    header = {};
    header.magic = net::magic;
    header.version = net::version;
    header.channels = (uint16_t) channels;
    header.senderId = id;
    header.session = sess;
    header.sampleRate = (uint32_t) std::lround (sr);
    header.frames = (uint16_t) fpp;
}

void Packetizer::finishPacket() noexcept
{
    const size_t blockBytes = blockA.size();
    header.firstFrame = nextFrame;
    header.flags = (redundant && havePrev) ? 1 : 0;
    header.prevFirstFrame = havePrev ? nextFrame - (juce::uint64) fpp : 0;

    std::memcpy (datagram.data(), &header, sizeof (header));
    std::memcpy (datagram.data() + net::headerSize, blockA.data(), blockBytes);
    datagramSize = net::headerSize + (int) blockBytes;

    if (header.flags & 1)
    {
        std::memcpy (datagram.data() + datagramSize, blockB.data(), blockBytes);
        datagramSize += (int) blockBytes;
    }

    std::swap (blockA, blockB);   // this block becomes the redundant copy in the next packet
    havePrev = true;
    nextFrame += (juce::uint64) fpp;
    filled = 0;
}

//==============================================================================
namespace
{
    struct LambdaThread : juce::Thread
    {
        LambdaThread (const juce::String& n, std::function<void()> f) : juce::Thread (n), fn (std::move (f)) {}
        void run() override { fn(); }
        std::function<void()> fn;
    };

    uint32_t randomNonZero()
    {
        juce::Random r;
        r.setSeedRandomly();
        uint32_t v = 0;
        while (v == 0)
            v = (uint32_t) r.nextInt64();
        return v;
    }
}

NetSender::NetSender() : senderId (randomNonZero())
{
    queue.reset (new QueuedPacket[queueSize]);
}

NetSender::~NetSender()
{
    stop();
}

bool NetSender::start (const juce::String& n, juce::String& error)
{
    setName (n);
    if (running)
        return true;

    socket = std::make_unique<juce::DatagramSocket> (true);
    if (! socket->bindToPort (net::controlPort) && ! socket->bindToPort (0))
    {
        error = "Could not open a network port.";
        socket.reset();
        return false;
    }
    boundPort = socket->getBoundPort();

    running = true;
    thread = std::make_unique<LambdaThread> ("Hawthorne Link TX", [this] { run(); });
    thread->startThread (juce::Thread::Priority::highest);
    return true;
}

void NetSender::stop()
{
    if (! running)
        return;
    running = false;
    wake.signal();
    if (socket != nullptr) socket->shutdown();
    if (thread != nullptr) thread->stopThread (2000);
    thread.reset();
    socket.reset();
}

void NetSender::setName (const juce::String& n)
{
    const juce::ScopedLock sl (lock);
    name = n.trim().isNotEmpty() ? n.trim() : juce::SystemStats::getComputerName();
}

juce::String NetSender::getName() const
{
    const juce::ScopedLock sl (lock);
    return name;
}

std::vector<NetSender::Subscriber> NetSender::getSubscribers() const
{
    const juce::ScopedLock sl (lock);
    return subscribers;
}

void NetSender::prepare (double sr, int ch)
{
    prepared = false;
    session = randomNonZero();
    channels = juce::jlimit (1, net::maxChannels, ch);
    sampleRate = sr;
    packetizer.prepare (senderId, session, sr, channels, redundancy);
    prepared = true;
}

void NetSender::push (const float* const* in, int numChannels, int n) noexcept
{
    if (! prepared.load (std::memory_order_relaxed))
        return;

    bool any = false;
    packetizer.push (in, numChannels, n, [this, &any] (const uint8_t* data, int size)
    {
        const int w = queueWrite.load (std::memory_order_relaxed);
        const int next = (w + 1) % queueSize;
        if (next == queueRead.load (std::memory_order_acquire))
        {
            overruns.fetch_add (1, std::memory_order_relaxed);
            return;
        }
        auto& q = queue[(size_t) w];
        std::memcpy (q.data.data(), data, (size_t) size);
        q.size = size;
        queueWrite.store (next, std::memory_order_release);
        any = true;
    });

    if (any)
        wake.signal();
}

//==============================================================================
void NetSender::announce (const juce::String& address, int port)
{
    net::Announce a;
    a.senderId = senderId;
    a.session = session;
    a.controlPort = boundPort;
    a.channels = channels;
    a.sampleRate = sampleRate;
    a.name = getName();
    const auto msg = net::makeAnnounce (a);
    socket->write (address, port, msg.toRawUTF8(), (int) msg.getNumBytesAsUTF8());
}

void NetSender::handleControl (const juce::String& msg, const juce::String& from, int port)
{
    auto t = juce::StringArray::fromTokens (msg.upToFirstOccurrenceOf ("\n", false, false), " ", {});
    if (t.size() < 2 || t[0] != "HWNA1")
        return;

    if (t[1] == "QRY")
    {
        announce (from, port);
        return;
    }

    if (t.size() < 3 || net::fromHex (t[2]) != senderId)
        return;

    const juce::ScopedLock sl (lock);
    const uint32_t rid = t.size() > 3 ? net::fromHex (t[3]) : 0;
    // One entry per receiver, however many addresses it reaches us from.
    auto it = std::find_if (subscribers.begin(), subscribers.end(), [&] (const Subscriber& s)
    {
        return rid != 0 ? s.receiverId == rid : (s.address == from && s.port == port);
    });

    if (t[1] == "SUB")
    {
        if (it == subscribers.end())
        {
            subscribers.push_back ({ from, port, 0, rid });
            it = subscribers.end() - 1;
        }
        it->address = from;
        it->port = port;
        it->lastSeenMs = juce::Time::getMillisecondCounter();
    }
    else if (t[1] == "BYE" && it != subscribers.end())
    {
        subscribers.erase (it);
    }
}

void NetSender::run()
{
    std::vector<uint8_t> buf (2048);
    std::vector<Subscriber> targets;
    juce::uint32 lastAnnounce = 0;
    auto* self = juce::Thread::getCurrentThread();

    while (running && self != nullptr && ! self->threadShouldExit())
    {
        wake.wait (2);

        // Control messages
        while (socket->waitUntilReady (true, 0) == 1)
        {
            juce::String from;
            int port = 0;
            const int n = socket->read (buf.data(), (int) buf.size(), false, from, port);
            if (n <= 0) break;
            handleControl (juce::String::fromUTF8 ((const char*) buf.data(), n), from, port);
        }

        const auto now = juce::Time::getMillisecondCounter();
        if (now - lastAnnounce >= 1000)
        {
            lastAnnounce = now;
            announce ("255.255.255.255", net::announcePort);   // seen even before its audio starts

            const juce::ScopedLock sl (lock);
            subscribers.erase (std::remove_if (subscribers.begin(), subscribers.end(),
                                               [now] (const Subscriber& s) { return now - s.lastSeenMs > (juce::uint32) net::subscriptionTimeoutMs; }),
                               subscribers.end());
        }

        {
            const juce::ScopedLock sl (lock);
            targets = subscribers;
        }

        // Audio
        for (;;)
        {
            const int r = queueRead.load (std::memory_order_relaxed);
            if (r == queueWrite.load (std::memory_order_acquire))
                break;

            const auto& q = queue[(size_t) r];
            for (auto& t : targets)
                socket->write (t.address, t.port, q.data.data(), q.size);

            queueRead.store ((r + 1) % queueSize, std::memory_order_release);
            packetsSent.fetch_add (1, std::memory_order_relaxed);
        }
    }
}

} // namespace hawthorne
