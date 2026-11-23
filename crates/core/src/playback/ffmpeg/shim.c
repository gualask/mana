/* The FFmpeg demuxer handle the Rust side drives (ffmpeg/mod.rs): opening a
 * file or a byte stream, reading packets, rewinding, closing. Its types are
 * in media.h; probe.c finds the video size, mp4.c prepares MP4 input, and
 * transcoder.c turns MPEG audio into AAC. */

#include "media.h"

#include <stdio.h>
#include <string.h>

#include "adts.h"

static int64_t to_us(int64_t ts, AVRational base) {
    return ts == AV_NOPTS_VALUE ? INT64_MIN : av_rescale_q(ts, base, AV_TIME_BASE_Q);
}

/* Duration of one audio frame in µs, read from its header; 0 when the
 * header is not recognized. Frames split from one MPEG-TS PES packet carry
 * no timestamp of their own: each continues from the previous one. */
static int64_t frame_duration_us(int codec, const uint8_t *d, int size) {
    int samples = 0, rate = 0;
    if (codec == MANA_CODEC_AAC && size >= 7 && d[0] == 0xFF && (d[1] & 0xF0) == 0xF0) {
        int index = (d[2] >> 2) & 0x0F;
        /* HE-AAC headers carry the core rate: 1024 core samples per frame. */
        if (index < 12) samples = 1024, rate = AAC_RATES[index];
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

/* Stream setup shared by files and callback streams, once fmt is open. */
static mana_media *finish_open(mana_media *m, mana_info *info, mana_error *err) {
    int r = avformat_find_stream_info(m->fmt, NULL);
    if (r < 0) {
        fail(err, MANA_STEP_STREAM_INFO, r);
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
    if (strstr(m->fmt->iformat->name, "mp4") && setup_mp4(m, err) < 0) {
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

mana_media *mana_media_open(const char *url, mana_info *info, mana_error *err) {
    mana_media *m = media_alloc(info);
    if (!m) {
        fail(err, MANA_STEP_OPEN, AVERROR(ENOMEM));
        return NULL;
    }
    AVDictionary *options = open_options();
    int r = avformat_open_input(&m->fmt, url, NULL, &options);
    av_dict_free(&options);
    if (r < 0) {
        fail(err, MANA_STEP_OPEN, r);
        mana_media_close(m);
        return NULL;
    }
    return finish_open(m, info, err);
}

/* Opens an MPEG-TS or fragmented MP4 byte stream pulled from `read`, which returns the bytes
 * copied, AVERROR_EOF at the end or AVERROR_EXIT to abort. */
mana_media *mana_media_open_stream(mana_read_fn read, void *opaque, mana_info *info, mana_error *err) {
    mana_media *m = media_alloc(info);
    if (!m) {
        fail(err, MANA_STEP_OPEN, AVERROR(ENOMEM));
        return NULL;
    }
    uint8_t *buffer = av_malloc(IO_BUFFER_SIZE);
    m->io = buffer ? avio_alloc_context(buffer, IO_BUFFER_SIZE, 0, opaque, read, NULL, NULL) : NULL;
    m->fmt = avformat_alloc_context();
    if (!m->io || !m->fmt) {
        if (!m->io) av_free(buffer);
        fail(err, MANA_STEP_OPEN, AVERROR(ENOMEM));
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
        fail(err, MANA_STEP_OPEN, r);
        mana_media_close(m);
        return NULL;
    }
    return finish_open(m, info, err);
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
            int r = transcoder_next(m->transcoder, &out->data, &out->size, &out->pts_us);
            if (r < 0) return r;
            if (r) {
                out->keyframe = 1;
                out->dts_us = out->pts_us;
                return MANA_AUDIO;
            }
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
            int sent = transcoder_send(m->transcoder, &m->packet);
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
    if (m->transcoder) transcoder_flush(m->transcoder);
    return av_seek_frame(m->fmt, -1, 0, AVSEEK_FLAG_BACKWARD);
}
