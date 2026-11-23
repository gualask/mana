/* MPEG audio to AAC in ADTS (transcoder.h). Decoded samples are queued and
 * cut into the encoder's 1024-sample frames; timestamps follow the sample
 * count from the first decoded frame and move with the source when it jumps
 * (a loop, a discontinuity). */

#include "transcoder.h"

#include <libavutil/audio_fifo.h>
#include <libavutil/channel_layout.h>
#include <stdlib.h>

#include "adts.h"

struct transcoder {
    AVCodecContext *decoder;
    AVCodecContext *encoder;
    AVAudioFifo *fifo;
    AVFrame *decoded;
    AVFrame *chunk;
    AVPacket *encoded;
    int rate;
    /* Timeline (µs) of encoder sample 0, samples sent to the encoder, and
     * where the next decoded frame should start. */
    int64_t base_us;
    int64_t samples;
    int64_t next_input_us;
    uint8_t adts[7 + 8192];
};

/* A decoded frame this far from where the previous one ended is a jump. */
#define TRANSCODE_JUMP_US 200000
#define AAC_BITRATE 128000

/* Channel counts and layouts: FFmpeg 5.1 added AVChannelLayout, 7.0 removed
 * the fields it replaced. */
#if LIBAVUTIL_VERSION_MAJOR >= 59
static int frame_channels(const AVFrame *f) { return f->ch_layout.nb_channels; }
static int encoder_channels(const AVCodecContext *c) { return c->ch_layout.nb_channels; }
static int set_encoder_layout(AVCodecContext *c, const AVFrame *f) {
    if (f->ch_layout.order == AV_CHANNEL_ORDER_UNSPEC) {
        av_channel_layout_default(&c->ch_layout, f->ch_layout.nb_channels);
        return 0;
    }
    return av_channel_layout_copy(&c->ch_layout, &f->ch_layout);
}
static int set_frame_layout(AVFrame *f, const AVCodecContext *c) { return av_channel_layout_copy(&f->ch_layout, &c->ch_layout); }
#else
static int frame_channels(const AVFrame *f) { return f->channels; }
static int encoder_channels(const AVCodecContext *c) { return c->channels; }
static int set_encoder_layout(AVCodecContext *c, const AVFrame *f) {
    c->channels = f->channels;
    c->channel_layout = f->channel_layout ? f->channel_layout : av_get_default_channel_layout(f->channels);
    return 0;
}
static int set_frame_layout(AVFrame *f, const AVCodecContext *c) {
    f->channel_layout = c->channel_layout;
    return 0;
}
#endif

void transcoder_free(transcoder *t) {
    if (!t) return;
    avcodec_free_context(&t->decoder);
    avcodec_free_context(&t->encoder);
    if (t->fifo) av_audio_fifo_free(t->fifo);
    av_frame_free(&t->decoded);
    av_frame_free(&t->chunk);
    av_packet_free(&t->encoded);
    av_free(t);
}

transcoder *transcoder_new(enum AVCodecID id) {
    /* The float decoders output planar float, which the AAC encoder takes. */
    const AVCodec *codec = avcodec_find_decoder_by_name(id == AV_CODEC_ID_MP2 ? "mp2float" : "mp3float");
    transcoder *t = av_mallocz(sizeof *t);
    if (!t || !codec) goto fail;
    t->decoder = avcodec_alloc_context3(codec);
    t->decoded = av_frame_alloc();
    t->chunk = av_frame_alloc();
    t->encoded = av_packet_alloc();
    if (!t->decoder || !t->decoded || !t->chunk || !t->encoded || avcodec_open2(t->decoder, codec, NULL) < 0)
        goto fail;
    t->base_us = INT64_MIN;
    return t;
fail:
    transcoder_free(t);
    return NULL;
}

/* Opens the encoder for the first decoded frame's format. */
static int transcoder_start(transcoder *t, const AVFrame *f) {
    const AVCodec *codec = avcodec_find_encoder(AV_CODEC_ID_AAC);
    if (!codec || f->format != AV_SAMPLE_FMT_FLTP) return AVERROR(EINVAL);
    t->encoder = avcodec_alloc_context3(codec);
    if (!t->encoder) return AVERROR(ENOMEM);
    t->rate = f->sample_rate;
    t->encoder->sample_fmt = AV_SAMPLE_FMT_FLTP;
    t->encoder->sample_rate = f->sample_rate;
    int r = set_encoder_layout(t->encoder, f);
    if (r < 0) return r;
    t->encoder->bit_rate = AAC_BITRATE;
    t->encoder->time_base = (AVRational){1, f->sample_rate};
    if ((r = avcodec_open2(t->encoder, codec, NULL)) < 0) return r;
    t->fifo = av_audio_fifo_alloc(AV_SAMPLE_FMT_FLTP, frame_channels(f), t->encoder->frame_size * 4);
    return t->fifo ? 0 : AVERROR(ENOMEM);
}

int transcoder_next(transcoder *t, const uint8_t **data, int *size, int64_t *pts_us) {
    for (;;) {
        if (t->encoder) {
            int r = avcodec_receive_packet(t->encoder, t->encoded);
            if (r == 0) {
                int length = write_adts(t->adts, sizeof t->adts, 2, aac_rate_index(t->rate), encoder_channels(t->encoder),
                                        t->encoded->data, t->encoded->size);
                int64_t pts = t->encoded->pts;
                av_packet_unref(t->encoded);
                if (length < 0) return length;
                *data = t->adts;
                *size = length;
                *pts_us = t->base_us + pts * 1000000 / t->rate;
                return 1;
            }
            if (r != AVERROR(EAGAIN)) return r;
            int frame_size = t->encoder->frame_size;
            if (av_audio_fifo_size(t->fifo) >= frame_size) {
                av_frame_unref(t->chunk);
                t->chunk->nb_samples = frame_size;
                t->chunk->format = AV_SAMPLE_FMT_FLTP;
                t->chunk->sample_rate = t->rate;
                if ((r = set_frame_layout(t->chunk, t->encoder)) < 0) return r;
                if ((r = av_frame_get_buffer(t->chunk, 0)) < 0) return r;
                av_audio_fifo_read(t->fifo, (void **)t->chunk->data, frame_size);
                t->chunk->pts = t->samples;
                t->samples += frame_size;
                if ((r = avcodec_send_frame(t->encoder, t->chunk)) < 0) return r;
                continue;
            }
        }
        int r = avcodec_receive_frame(t->decoder, t->decoded);
        if (r == AVERROR(EAGAIN)) return 0;
        if (r < 0) return r;
        if (!t->encoder && (r = transcoder_start(t, t->decoded)) < 0) return r;
        AVFrame *f = t->decoded;
        int64_t frame_us = f->best_effort_timestamp; /* µs: see transcoder_send */
        if (t->base_us == INT64_MIN) {
            t->base_us = frame_us == AV_NOPTS_VALUE ? 0 : frame_us;
        } else if (frame_us != AV_NOPTS_VALUE && llabs(frame_us - t->next_input_us) > TRANSCODE_JUMP_US) {
            /* Follow the source: shift what is still to be encoded. */
            t->base_us += frame_us - t->next_input_us;
        }
        t->next_input_us = (frame_us == AV_NOPTS_VALUE ? t->next_input_us : frame_us) +
                           (int64_t)f->nb_samples * 1000000 / t->rate;
        av_audio_fifo_write(t->fifo, (void **)f->extended_data, f->nb_samples);
        av_frame_unref(f);
    }
}

int transcoder_send(transcoder *t, const AVPacket *packet) {
    return avcodec_send_packet(t->decoder, packet);
}

void transcoder_flush(transcoder *t) {
    avcodec_flush_buffers(t->decoder);
    if (t->fifo) av_audio_fifo_reset(t->fifo);
}
