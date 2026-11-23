/* MPEG audio (Layer II, MP3) to AAC in ADTS, for the TV's pipeline, which
 * does not take MPEG audio as a stream. */

#ifndef MANA_TRANSCODER_H
#define MANA_TRANSCODER_H

#include <libavcodec/avcodec.h>
#include <stdint.h>

typedef struct transcoder transcoder;

/* A transcoder for the MPEG audio codec `id`; NULL when this FFmpeg cannot
 * decode it. */
transcoder *transcoder_new(enum AVCodecID id);
void transcoder_free(transcoder *t);
/* Hands a packet, timestamps in µs, to the decoder, as
 * avcodec_send_packet. */
int transcoder_send(transcoder *t, const AVPacket *packet);
/* Produces the next AAC frame from what was decoded so far: 1 with `data`,
 * `size` and `pts_us` filled, valid until the next call, 0 when more input
 * is needed, or a negative AVERROR. */
int transcoder_next(transcoder *t, const uint8_t **data, int *size, int64_t *pts_us);
/* Drops what is decoded and queued, before a seek. */
void transcoder_flush(transcoder *t);

#endif
