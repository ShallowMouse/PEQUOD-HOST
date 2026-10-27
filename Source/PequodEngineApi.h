#pragma once

#include <cstdint>

//==============================================================================
/** The PEQUOD engine's C interface: every PEQUOD plugin, built from the same
    Rust source as the VST3 versions and linked straight into the host
    (engine/ in this repo; see engine/README.md).

    Threads: pq_set_param / pq_get_param / pq_param_text / pq_meters /
    pq_get_state / pq_set_state are safe from any thread. pq_prepare,
    pq_reset, pq_midi and pq_process are the audio side: one thread at a time,
    and never pq_prepare/pq_reset while pq_process runs. */
extern "C"
{
    struct PqHandle;

    int32_t     pq_count();
    const char* pq_info (int32_t index);   // JSON, owned by the engine
    const char* pq_panel (int32_t index);  // JSON, owned by the engine

    PqHandle* pq_create (int32_t index);
    void      pq_destroy (PqHandle*);

    void    pq_set_param (const PqHandle*, int32_t param, float normalised);
    float   pq_get_param (const PqHandle*, int32_t param);
    int32_t pq_param_text (const PqHandle*, int32_t param, float normalised, char* buffer, int32_t length);
    int32_t pq_param_from_text (const PqHandle*, int32_t param, const char* text, float* normalisedOut);
    float   pq_normalize (const PqHandle*, int32_t param, float plain);

    uint32_t pq_prepare (const PqHandle*, double sampleRate, int32_t channels, int32_t maxBlock);
    void     pq_reset (const PqHandle*);
    /** kind: 0 note on (a = note, b = velocity 0..1), 1 note off, 2 CC (b = 0..1), 3 pitch bend (b = -1..1), 4 all notes off. */
    void     pq_midi (const PqHandle*, int32_t sampleOffset, int32_t kind, int32_t a, float b);
    /** flags: 1 tempo valid, 2 beats valid, 4 playing. */
    void     pq_process (PqHandle*, float* const* channels, int32_t numChannels, int32_t numSamples,
                         double tempo, double beats, int32_t flags);
    int32_t  pq_meters (const PqHandle*, float* out, int32_t max);

    int32_t pq_get_state (const PqHandle*, char* buffer, int32_t length);
    int32_t pq_set_state (const PqHandle*, const uint8_t* data, int32_t length);
}
