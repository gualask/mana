/* ADTS headers for raw AAC frames: the TV's pipeline takes AAC in ADTS, as
 * MPEG-TS carries it, so the AAC of an MP4 and the AAC the transcoder makes
 * get one. */

#ifndef MANA_ADTS_H
#define MANA_ADTS_H

#include <libavutil/error.h>
#include <stdint.h>
#include <string.h>

static const int AAC_RATES[] = {96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000};

static inline int aac_rate_index(int rate) {
    for (int i = 0; i < 12; i++)
        if (AAC_RATES[i] == rate) return i;
    return 3; /* 48 kHz */
}

/* Writes one raw AAC frame with an ADTS header into `out`; returns its
 * length, or a negative AVERROR when it does not fit. `object_type` is the
 * MPEG-4 audio object type (2: AAC LC). */
static inline int write_adts(uint8_t *out, int capacity, int object_type, int rate_index, int channels,
                             const uint8_t *payload, int size) {
    int length = size + 7;
    if (length > capacity || length > 0x1FFF) return AVERROR(EINVAL);
    out[0] = 0xFF;
    out[1] = 0xF1; /* MPEG-4, no CRC */
    out[2] = (uint8_t)(((object_type - 1) << 6) | (rate_index << 2) | (channels >> 2));
    out[3] = (uint8_t)(((channels & 3) << 6) | (length >> 11));
    out[4] = (uint8_t)((length >> 3) & 0xFF);
    out[5] = (uint8_t)(((length & 7) << 5) | 0x1F);
    out[6] = 0xFC;
    memcpy(out + 7, payload, size);
    return length;
}

#endif
