/* Thin wrapper over FFmpeg, the one bundled with the app on the TV
 * (tools/webos/ffmpeg.py) or the desktop's: demuxes a file, or an MPEG-TS or fMP4 byte
 * stream pulled through a read callback, into video access units and audio
 * frames, both handed to the TV's decoders as they are.
 * Compiled against the matching upstream headers, so FFmpeg's struct layouts
 * never cross into Rust. */

#include <libavcodec/avcodec.h>
#if LIBAVCODEC_VERSION_MAJOR >= 59
#include <libavcodec/bsf.h> /* no longer included by avcodec.h */
#endif
#include <libavformat/avformat.h>
#include <libavutil/audio_fifo.h>
#include <libavutil/channel_layout.h>
#include <stdio.h>
#include <string.h>

/* Packets read ahead while looking for the video size. */
#define PROBE_PACKETS 256
#define DEFAULT_RATE 48000
/* LG's FFmpeg 4.0 aborts (aviobuf.c:581, "len >= s->orig_buffer_size") when
 * it shrinks a stream buffer that MPEG-TS probing enlarged to the probe size
 * plus a margin. A stream buffer larger than that is never shrunk. */
#define PROBE_SIZE (512 * 1024)
#define IO_BUFFER_SIZE (1024 * 1024)
#define ANALYZE_US 500000

enum { MANA_EOF = 0, MANA_VIDEO = 1, MANA_AUDIO = 2 };
enum {
    MANA_CODEC_OTHER = 0,
    MANA_CODEC_H264 = 1,
    MANA_CODEC_HEVC = 2,
    MANA_CODEC_AAC = 3,
    MANA_CODEC_AC3 = 6,
    MANA_CODEC_EAC3 = 7,
};

typedef struct mana_info {
    int width;
    int height;
    int video_codec;
    /* An audio MANA_CODEC_*, or -1 without an audio stream (same for video). */
    int audio_codec;
    char video_name[32];
    char audio_name[32];
} mana_info;

typedef struct mana_packet {
    const uint8_t *data;
    int size;
    int keyframe;
    /* Microseconds in the source timeline; INT64_MIN when unknown. */
    int64_t pts_us;
    int64_t dts_us;
} mana_packet;

typedef int (*mana_read_fn)(void *opaque, uint8_t *buf, int size);

typedef struct mana_media {
    AVFormatContext *fmt;
    /* Custom I/O for streams; owned here, not by fmt. */
    AVIOContext *io;
    int video_index;
    int audio_index;
    AVPacket packet;
    int packet_held;
    AVPacket *queue[PROBE_PACKETS];
    int queue_head;
    int queue_count;
    /* Where the next audio frame starts when it carries no timestamp. */
    int64_t next_audio_us;
    int audio_codec;
    /* MPEG audio, which the TV's pipeline does not take as a stream, is
     * transcoded to AAC here. */
    struct transcoder *transcoder;
    /* MP4 input (fMP4 HLS): video goes through mp4toannexb, AAC gets ADTS
     * headers built from its AudioSpecificConfig. */
    AVBSFContext *annexb;
    AVPacket *filtered;
    int filtered_held;
    int aac_object_type; /* 0 unless raw AAC needs ADTS headers */
    int aac_rate_index;
    int aac_channels;
    uint8_t adts[7 + 8192];
} mana_media;

/* MPEG audio (Layer II, MP3) to AAC in ADTS. Decoded samples are queued and
 * cut into the encoder's 1024-sample frames; timestamps follow the sample
 * count from the first decoded frame and move with the source when it jumps
 * (a loop, a discontinuity). */
typedef struct transcoder {
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
} transcoder;

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

static void error_text(int code, char *out, int out_size, const char *what) {
    char reason[128];
    av_strerror(code, reason, sizeof reason);
    snprintf(out, out_size, "%s: %s", what, reason);
}

static int64_t to_us(int64_t ts, AVRational base) {
    return ts == AV_NOPTS_VALUE ? INT64_MIN : av_rescale_q(ts, base, AV_TIME_BASE_Q);
}

/* Duration of one audio frame in µs, read from its header; 0 when the
 * header is not recognized. Frames split from one MPEG-TS PES packet carry
 * no timestamp of their own: each continues from the previous one. */
static int64_t frame_duration_us(int codec, const uint8_t *d, int size) {
    int samples = 0, rate = 0;
    if (codec == MANA_CODEC_AAC && size >= 7 && d[0] == 0xFF && (d[1] & 0xF0) == 0xF0) {
        static const int rates[] = {96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000};
        int index = (d[2] >> 2) & 0x0F;
        /* HE-AAC headers carry the core rate: 1024 core samples per frame. */
        if (index < 12) samples = 1024, rate = rates[index];
    } else if ((codec == MANA_CODEC_AC3 || codec == MANA_CODEC_EAC3) && size >= 6 && d[0] == 0x0B && d[1] == 0x77) {
        static const int rates[] = {48000, 44100, 32000}, half[] = {24000, 22050, 16000}, blocks[] = {1, 2, 3, 6};
        int fscod = d[4] >> 6, bsid = d[5] >> 3;
        if (bsid <= 10) { /* AC-3 */
            if (fscod < 3) samples = 1536, rate = rates[fscod];
        } else if (fscod < 3) { /* E-AC-3 */
            samples = 256 * blocks[(d[4] >> 4) & 3], rate = rates[fscod];
        } else {
            samples = 256 * 6, rate = half[((d[4] >> 4) & 3) % 3];
        }
    }
    return rate ? samples * 1000000LL / rate : 0;
}

static void clear_queue(mana_media *m) {
    while (m->queue_count) {
        av_packet_free(&m->queue[m->queue_head]);
        m->queue_head = (m->queue_head + 1) % PROBE_PACKETS;
        m->queue_count--;
    }
    m->queue_head = 0;
}

/* Next demuxed packet: queued probe packets first, then the demuxer. */
static int next_packet(mana_media *m, AVPacket *out) {
    if (m->queue_count) {
        av_packet_move_ref(out, m->queue[m->queue_head]);
        av_packet_free(&m->queue[m->queue_head]);
        m->queue_head = (m->queue_head + 1) % PROBE_PACKETS;
        m->queue_count--;
        return 0;
    }
    return av_read_frame(m->fmt, out);
}

/* Reads ahead, keeping every packet for playback, until the video parser
 * has seen a sequence header. */
static void probe_video_size(mana_media *m, mana_info *info) {
    AVCodecParameters *par = m->fmt->streams[m->video_index]->codecpar;
    AVCodecParserContext *parser = av_parser_init(par->codec_id);
    AVCodecContext *ctx = avcodec_alloc_context3(NULL);
    if (!parser || !ctx) goto done;
    parser->flags |= PARSER_FLAG_COMPLETE_FRAMES;
    while (!info->width && m->queue_count < PROBE_PACKETS) {
        AVPacket *packet = av_packet_alloc();
        if (!packet || av_read_frame(m->fmt, packet) < 0) {
            av_packet_free(&packet);
            break;
        }
        m->queue[m->queue_count++] = packet;
        if (packet->stream_index != m->video_index) continue;
        uint8_t *frame;
        int frame_size;
        av_parser_parse2(parser, ctx, &frame, &frame_size, packet->data, packet->size, packet->pts, packet->dts,
                         packet->pos);
        if (parser->width > 0 && parser->height > 0) {
            info->width = parser->width;
            info->height = parser->height;
        }
    }
done:
    av_parser_close(parser);
    avcodec_free_context(&ctx);
}

static void transcoder_free(transcoder *t) {
    if (!t) return;
    avcodec_free_context(&t->decoder);
    avcodec_free_context(&t->encoder);
    if (t->fifo) av_audio_fifo_free(t->fifo);
    av_frame_free(&t->decoded);
    av_frame_free(&t->chunk);
    av_packet_free(&t->encoded);
    av_free(t);
}

static transcoder *transcoder_new(enum AVCodecID id) {
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

static const int AAC_RATES[] = {96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000};

static int aac_rate_index(int rate) {
    for (int i = 0; i < 12; i++)
        if (AAC_RATES[i] == rate) return i;
    return 3; /* 48 kHz */
}

/* Writes one raw AAC frame with an ADTS header into `out`; returns its
 * length, or a negative AVERROR when it does not fit. `object_type` is the
 * MPEG-4 audio object type (2: AAC LC). */
static int write_adts(uint8_t *out, int capacity, int object_type, int rate_index, int channels,
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

/* Produces the next AAC frame from what was decoded so far: 1 with `out`
 * filled, 0 when more input is needed, or a negative AVERROR. */
static int transcoder_next(transcoder *t, mana_packet *out) {
    for (;;) {
        if (t->encoder) {
            int r = avcodec_receive_packet(t->encoder, t->encoded);
            if (r == 0) {
                int length = write_adts(t->adts, sizeof t->adts, 2, aac_rate_index(t->rate), encoder_channels(t->encoder),
                                        t->encoded->data, t->encoded->size);
                int64_t pts = t->encoded->pts;
                av_packet_unref(t->encoded);
                if (length < 0) return length;
                out->data = t->adts;
                out->size = length;
                out->keyframe = 1;
                out->pts_us = out->dts_us = t->base_us + pts * 1000000 / t->rate;
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
        int64_t frame_us = f->best_effort_timestamp; /* µs: see mana_media_read */
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

void mana_media_close(mana_media *m) {
    if (!m) return;
    transcoder_free(m->transcoder);
    av_bsf_free(&m->annexb);
    av_packet_free(&m->filtered);
    clear_queue(m);
    if (m->packet_held) av_packet_unref(&m->packet);
    avformat_close_input(&m->fmt);
    if (m->io) {
        av_freep(&m->io->buffer);
        avio_context_free(&m->io);
    }
    av_free(m);
}

static int find_stream(AVFormatContext *fmt, enum AVMediaType type) {
    for (unsigned i = 0; i < fmt->nb_streams; i++)
        if (fmt->streams[i]->codecpar->codec_type == type) return (int)i;
    return -1;
}

/* MP4 carries length-prefixed NAL units and raw AAC; the TV takes Annex B
 * and ADTS, as in MPEG-TS. */
static int setup_mp4(mana_media *m, char *err, int err_size) {
    AVStream *video = m->video_index >= 0 ? m->fmt->streams[m->video_index] : NULL;
    const char *name = !video                                          ? NULL
                       : video->codecpar->codec_id == AV_CODEC_ID_H264 ? "h264_mp4toannexb"
                       : video->codecpar->codec_id == AV_CODEC_ID_HEVC ? "hevc_mp4toannexb"
                                                                       : NULL;
    const AVBitStreamFilter *filter = name ? av_bsf_get_by_name(name) : NULL;
    if (filter) {
        int r = av_bsf_alloc(filter, &m->annexb);
        if (r >= 0) r = avcodec_parameters_copy(m->annexb->par_in, video->codecpar);
        if (r >= 0) m->annexb->time_base_in = video->time_base;
        if (r >= 0) r = av_bsf_init(m->annexb);
        m->filtered = av_packet_alloc();
        if (r < 0 || !m->filtered) {
            error_text(r < 0 ? r : AVERROR(ENOMEM), err, err_size, "mp4toannexb");
            return -1;
        }
    }
    if (m->audio_index >= 0) {
        AVCodecParameters *audio = m->fmt->streams[m->audio_index]->codecpar;
        if (audio->codec_id == AV_CODEC_ID_AAC) {
            /* AudioSpecificConfig: object type (5 bits), rate index (4),
             * channel configuration (4). SBR/PS (HE-AAC) signal the core. */
            const uint8_t *c = audio->extradata;
            if (audio->extradata_size < 2) {
                snprintf(err, err_size, "AAC senza configurazione nel MP4");
                return -1;
            }
            int object_type = c[0] >> 3, rate_index = ((c[0] & 7) << 1) | (c[1] >> 7);
            m->aac_object_type = object_type == 5 || object_type == 29 ? 2 : object_type;
            m->aac_rate_index = rate_index < 12 ? rate_index : aac_rate_index(audio->sample_rate);
            m->aac_channels = (c[1] >> 3) & 0x0F;
        }
    }
    return 0;
}

/* Stream setup shared by files and callback streams, once fmt is open. */
static mana_media *finish_open(mana_media *m, mana_info *info, char *err, int err_size) {
    int r = avformat_find_stream_info(m->fmt, NULL);
    if (r < 0) {
        error_text(r, err, err_size, "stream info");
        mana_media_close(m);
        return NULL;
    }
    /* av_find_best_stream() skips audio with 0 channels, which is every
     * audio stream on this build. */
    m->video_index = find_stream(m->fmt, AVMEDIA_TYPE_VIDEO);
    m->audio_index = find_stream(m->fmt, AVMEDIA_TYPE_AUDIO);
    /* An HLS audio rendition has no video: the caller decides if it needs one. */
    info->video_codec = -1;
    if (m->video_index >= 0) {
        AVCodecParameters *video = m->fmt->streams[m->video_index]->codecpar;
        info->width = video->width;
        info->height = video->height;
        switch (video->codec_id) {
        case AV_CODEC_ID_H264: info->video_codec = MANA_CODEC_H264; break;
        case AV_CODEC_ID_HEVC: info->video_codec = MANA_CODEC_HEVC; break;
        default: info->video_codec = MANA_CODEC_OTHER;
        }
        snprintf(info->video_name, sizeof info->video_name, "%s", avcodec_get_name(video->codec_id));
    }
    info->audio_codec = -1;
    if (m->audio_index >= 0) {
        AVCodecParameters *audio = m->fmt->streams[m->audio_index]->codecpar;
        switch (audio->codec_id) {
        case AV_CODEC_ID_AAC: info->audio_codec = MANA_CODEC_AAC; break;
        case AV_CODEC_ID_AC3: info->audio_codec = MANA_CODEC_AC3; break;
        case AV_CODEC_ID_EAC3: info->audio_codec = MANA_CODEC_EAC3; break;
        case AV_CODEC_ID_MP2:
        case AV_CODEC_ID_MP3:
            m->transcoder = transcoder_new(audio->codec_id);
            info->audio_codec = m->transcoder ? MANA_CODEC_AAC : MANA_CODEC_OTHER;
            break;
        default: info->audio_codec = MANA_CODEC_OTHER;
        }
        m->audio_codec = info->audio_codec;
        snprintf(info->audio_name, sizeof info->audio_name, "%s%s", avcodec_get_name(audio->codec_id),
                 m->transcoder ? " → aac" : "");
    }
    if (strstr(m->fmt->iformat->name, "mp4") && setup_mp4(m, err, err_size) < 0) {
        mana_media_close(m);
        return NULL;
    }
    if (m->video_index >= 0 && !info->width) probe_video_size(m, info);
    return m;
}

/* Demuxer options: raw timestamps, because FFmpeg's own 33-bit wrap
 * correction misfires after a discontinuity (it corrects DTS but not PTS);
 * the feeder unwraps each stream itself. A bounded probe size keeps the
 * stream buffer from being resized (see IO_BUFFER_SIZE). A short analysis:
 * on MPEG-TS FFmpeg otherwise reads 7 s of stream, which a live stream at
 * its start does not have yet (Canale 5's audio, 96 kb/s, waited for it);
 * what the analysis leaves out comes from the first packets (see
 * probe_video_size). */
static AVDictionary *open_options(void) {
    AVDictionary *options = NULL;
    av_dict_set(&options, "correct_ts_overflow", "0", 0);
    av_dict_set_int(&options, "probesize", PROBE_SIZE, 0);
    av_dict_set_int(&options, "analyzeduration", ANALYZE_US, 0);
    return options;
}

static mana_media *media_alloc(mana_info *info) {
    /* Warnings repeat per packet on damaged streams; errors are enough. */
    av_log_set_level(AV_LOG_ERROR);
    memset(info, 0, sizeof *info);
    mana_media *m = av_mallocz(sizeof *m);
    if (m) m->next_audio_us = INT64_MIN;
    return m;
}

mana_media *mana_media_open(const char *url, mana_info *info, char *err, int err_size) {
    mana_media *m = media_alloc(info);
    if (!m) return NULL;
    AVDictionary *options = open_options();
    int r = avformat_open_input(&m->fmt, url, NULL, &options);
    av_dict_free(&options);
    if (r < 0) {
        error_text(r, err, err_size, "open");
        mana_media_close(m);
        return NULL;
    }
    return finish_open(m, info, err, err_size);
}

/* Opens an MPEG-TS or fragmented MP4 byte stream pulled from `read`, which returns the bytes
 * copied, AVERROR_EOF at the end or AVERROR_EXIT to abort. */
mana_media *mana_media_open_stream(mana_read_fn read, void *opaque, mana_info *info, char *err, int err_size) {
    mana_media *m = media_alloc(info);
    if (!m) return NULL;
    uint8_t *buffer = av_malloc(IO_BUFFER_SIZE);
    m->io = buffer ? avio_alloc_context(buffer, IO_BUFFER_SIZE, 0, opaque, read, NULL, NULL) : NULL;
    m->fmt = avformat_alloc_context();
    if (!m->io || !m->fmt) {
        if (!m->io) av_free(buffer);
        snprintf(err, err_size, "out of memory");
        mana_media_close(m);
        return NULL;
    }
    m->fmt->pb = m->io;
    m->fmt->flags |= AVFMT_FLAG_CUSTOM_IO;
    AVDictionary *options = open_options();
    /* MPEG-TS or fragmented MP4, recognized from the first bytes. */
    int r = avformat_open_input(&m->fmt, NULL, NULL, &options);
    av_dict_free(&options);
    if (r < 0) {
        error_text(r, err, err_size, "open stream");
        mana_media_close(m);
        return NULL;
    }
    return finish_open(m, info, err, err_size);
}

/* Returns MANA_VIDEO or MANA_AUDIO with `out` valid until the next call,
 * MANA_EOF at the end, or a negative AVERROR. */
int mana_media_read(mana_media *m, mana_packet *out) {
    if (m->packet_held) {
        av_packet_unref(&m->packet);
        m->packet_held = 0;
    }
    if (m->filtered_held) {
        av_packet_unref(m->filtered);
        m->filtered_held = 0;
    }
    for (;;) {
        if (m->transcoder) {
            int r = transcoder_next(m->transcoder, out);
            if (r < 0) return r;
            if (r) return MANA_AUDIO;
        }
        int r = next_packet(m, &m->packet);
        if (r == AVERROR_EOF) return MANA_EOF;
        if (r < 0) return r;
        int index = m->packet.stream_index;
        if (index != m->video_index && index != m->audio_index) {
            av_packet_unref(&m->packet);
            continue;
        }
        AVRational base = m->fmt->streams[index]->time_base;
        m->packet_held = 1;
        out->data = m->packet.data;
        out->size = m->packet.size;
        out->pts_us = to_us(m->packet.pts, base);
        out->dts_us = to_us(m->packet.dts, base);
        if (index == m->video_index) {
            out->keyframe = (m->packet.flags & AV_PKT_FLAG_KEY) != 0;
            if (m->annexb) {
                /* One packet in, one out: the filter only rewrites it. */
                m->packet_held = 0;
                if ((r = av_bsf_send_packet(m->annexb, &m->packet)) < 0) return r;
                if ((r = av_bsf_receive_packet(m->annexb, m->filtered)) < 0) {
                    if (r == AVERROR(EAGAIN)) continue;
                    return r;
                }
                m->filtered_held = 1;
                out->data = m->filtered->data;
                out->size = m->filtered->size;
            }
            return MANA_VIDEO;
        }
        if (m->aac_object_type) {
            int length = write_adts(m->adts, sizeof m->adts, m->aac_object_type, m->aac_rate_index,
                                    m->aac_channels, m->packet.data, m->packet.size);
            if (length < 0) continue; /* oversized frame: skip it */
            out->data = m->adts;
            out->size = length;
        }
        if (m->transcoder) {
            /* Timestamps in µs, so decoded frames carry µs too. */
            m->packet.pts = out->pts_us == INT64_MIN ? AV_NOPTS_VALUE : out->pts_us;
            m->packet.dts = m->packet.pts;
            int sent = avcodec_send_packet(m->transcoder->decoder, &m->packet);
            av_packet_unref(&m->packet);
            m->packet_held = 0;
            if (sent < 0 && sent != AVERROR(EAGAIN)) continue; /* corrupt frame: skip it */
            continue;
        }
        /* A TS PES packet holds several audio frames and only the first is
         * stamped: the others continue from the previous frame's end. */
        out->keyframe = 1;
        if (out->pts_us == INT64_MIN) out->pts_us = m->next_audio_us;
        out->dts_us = out->pts_us;
        int64_t duration = frame_duration_us(m->audio_codec, m->packet.data, m->packet.size);
        m->next_audio_us = out->pts_us == INT64_MIN || duration == 0 ? INT64_MIN : out->pts_us + duration;
        return MANA_AUDIO;
    }
}

void mana_error_text(int code, char *out, int out_size) {
    av_strerror(code, out, out_size);
}

/* Restarts from the beginning, for looping a local file. */
int mana_media_rewind(mana_media *m) {
    if (m->packet_held) {
        av_packet_unref(&m->packet);
        m->packet_held = 0;
    }
    clear_queue(m);
    m->next_audio_us = INT64_MIN;
    if (m->transcoder) {
        transcoder *t = m->transcoder;
        avcodec_flush_buffers(t->decoder);
        if (t->fifo) av_audio_fifo_reset(t->fifo);
    }
    return av_seek_frame(m->fmt, -1, 0, AVSEEK_FLAG_BACKWARD);
}
