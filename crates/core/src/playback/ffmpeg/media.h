#ifndef MANA_MEDIA_H
#define MANA_MEDIA_H

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

#include "transcoder.h"

/* Packets read ahead while looking for the video size. */
#define PROBE_PACKETS 256
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

/* The step of opening that failed, with FFmpeg's code (0 when FFmpeg did
 * not fail): the caller words it. */
enum {
    MANA_STEP_OPEN = 1,
    MANA_STEP_STREAM_INFO = 2,
    MANA_STEP_ANNEXB = 3,
    MANA_STEP_AAC_CONFIG = 4,
};

typedef struct mana_error {
    int step;
    int code;
} mana_error;

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
    transcoder *transcoder;
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

static inline void fail(mana_error *err, int step, int code) {
    err->step = step;
    err->code = code;
}

/* probe.c: the packets read ahead while looking for the video size, then
 * handed out before the demuxer's. */
void clear_queue(mana_media *m);
int next_packet(mana_media *m, AVPacket *out);
void probe_video_size(mana_media *m, mana_info *info);

/* mp4.c: Annex B video and ADTS for the AAC of MP4 input. */
int setup_mp4(mana_media *m, mana_error *err);

#endif
