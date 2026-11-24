// Software decoding for the desktop backend: the access units the feeder
// hands over (H.264 or HEVC in Annex B, AAC in ADTS, AC-3, E-AC-3) to NV12
// pictures and to interleaved stereo float samples at the output's rate.
// Built only with the core's `decoder` feature; the TV decodes in Starfish.

#include <libavcodec/avcodec.h>
#include <libavutil/channel_layout.h>
#include <libavutil/imgutils.h>
#include <libavutil/opt.h>
#include <libswresample/swresample.h>
#include <libswscale/swscale.h>
#include <string.h>

#include "media.h"

typedef struct {
    AVCodecContext *ctx;
    AVPacket *packet;
    AVFrame *frame;
    struct SwsContext *sws;
    SwrContext *swr;
    int swr_rate;
} MDecoder;

// The codecs, numbered as MANA_CODEC_* in the shim.
static enum AVCodecID codec_id(int codec) {
    switch (codec) {
    case MANA_CODEC_H264: return AV_CODEC_ID_H264;
    case MANA_CODEC_HEVC: return AV_CODEC_ID_HEVC;
    case MANA_CODEC_AAC: return AV_CODEC_ID_AAC;
    case MANA_CODEC_AC3: return AV_CODEC_ID_AC3;
    case MANA_CODEC_EAC3: return AV_CODEC_ID_EAC3;
    default: return AV_CODEC_ID_NONE;
    }
}

void mana_decoder_close(MDecoder *d) {
    if (!d) return;
    avcodec_free_context(&d->ctx);
    av_packet_free(&d->packet);
    av_frame_free(&d->frame);
    sws_freeContext(d->sws);
    swr_free(&d->swr);
    av_free(d);
}

MDecoder *mana_decoder_open(int codec_number, int *error) {
    const AVCodec *codec = avcodec_find_decoder(codec_id(codec_number));
    if (!codec) {
        *error = AVERROR_DECODER_NOT_FOUND;
        return NULL;
    }
    MDecoder *d = av_mallocz(sizeof *d);
    if (!d) {
        *error = AVERROR(ENOMEM);
        return NULL;
    }
    d->ctx = avcodec_alloc_context3(codec);
    d->packet = av_packet_alloc();
    d->frame = av_frame_alloc();
    if (!d->ctx || !d->packet || !d->frame) {
        mana_decoder_close(d);
        *error = AVERROR(ENOMEM);
        return NULL;
    }
    // Timestamps travel in nanoseconds, as the feeder gives them.
    d->ctx->pkt_timebase = (AVRational){1, 1000000000};
    d->ctx->thread_count = 0;
    int r = avcodec_open2(d->ctx, codec, NULL);
    if (r < 0) {
        mana_decoder_close(d);
        *error = r;
        return NULL;
    }
    return d;
}

int mana_decoder_send(MDecoder *d, const uint8_t *data, int size, int64_t pts_ns) {
    d->packet->data = (uint8_t *)data;
    d->packet->size = size;
    d->packet->pts = pts_ns;
    d->packet->dts = AV_NOPTS_VALUE;
    int r = avcodec_send_packet(d->ctx, d->packet);
    d->packet->data = NULL;
    d->packet->size = 0;
    // 0: the decoder wants its frames read first, then the unit again. A
    // damaged unit is dropped, as a decoder in a TV drops it.
    if (r == AVERROR(EAGAIN)) return 0;
    return r == AVERROR_INVALIDDATA || r >= 0 ? 1 : r;
}

// The end of the units: the frames held back come out.
int mana_decoder_drain(MDecoder *d) {
    int r = avcodec_send_packet(d->ctx, NULL);
    return r == AVERROR_EOF ? 0 : r;
}

// 1 when a frame came out, 0 when the decoder wants more data, < 0 on error.
int mana_decoder_receive(MDecoder *d, int *width, int *height, int64_t *pts_ns, int *samples) {
    int r = avcodec_receive_frame(d->ctx, d->frame);
    if (r == AVERROR(EAGAIN) || r == AVERROR_EOF) return 0;
    if (r < 0) return r;
    *width = d->frame->width & ~1;
    *height = d->frame->height & ~1;
    *pts_ns = d->frame->best_effort_timestamp;
    *samples = d->frame->nb_samples;
    return 1;
}

// The frame received last as NV12 of its size rounded down to even numbers.
int mana_decoder_nv12(MDecoder *d, uint8_t *luma, uint8_t *chroma) {
    AVFrame *f = d->frame;
    int width = f->width & ~1, height = f->height & ~1;
    d->sws = sws_getCachedContext(d->sws, f->width, f->height, f->format, width, height, AV_PIX_FMT_NV12,
                                  SWS_BILINEAR, NULL, NULL, NULL);
    if (!d->sws) return AVERROR(EINVAL);
    uint8_t *planes[4] = {luma, chroma, NULL, NULL};
    int strides[4] = {width, width, 0, 0};
    int r = sws_scale(d->sws, (const uint8_t *const *)f->data, f->linesize, 0, f->height, planes, strides);
    av_frame_unref(f);
    return r < 0 ? r : 0;
}

// The frame received last as interleaved stereo floats at `rate`: at most
// `capacity` frames of two samples into `out`; how many were written.
int mana_decoder_stereo(MDecoder *d, int rate, float *out, int capacity) {
    AVFrame *f = d->frame;
    if (!d->swr || d->swr_rate != rate) {
        swr_free(&d->swr);
        AVChannelLayout stereo = AV_CHANNEL_LAYOUT_STEREO;
        int r = swr_alloc_set_opts2(&d->swr, &stereo, AV_SAMPLE_FMT_FLT, rate, &f->ch_layout, f->format,
                                    f->sample_rate, 0, NULL);
        if (r < 0 || (r = swr_init(d->swr)) < 0) {
            av_frame_unref(f);
            return r < 0 ? r : AVERROR(EINVAL);
        }
        d->swr_rate = rate;
    }
    uint8_t *planes[1] = {(uint8_t *)out};
    int r = swr_convert(d->swr, planes, capacity, (const uint8_t **)f->extended_data, f->nb_samples);
    av_frame_unref(f);
    return r;
}

// Room `mana_decoder_stereo` may need for the frame received last.
int mana_decoder_stereo_capacity(MDecoder *d, int rate) {
    AVFrame *f = d->frame;
    int64_t delay = d->swr ? swr_get_delay(d->swr, f->sample_rate) : 0;
    return (int)av_rescale_rnd(delay + f->nb_samples, rate, f->sample_rate > 0 ? f->sample_rate : rate, AV_ROUND_UP) + 32;
}
