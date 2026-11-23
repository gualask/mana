/* MP4 input (fMP4 HLS): the TV takes the video as Annex B and AAC with
 * ADTS headers, as in MPEG-TS (media.h). */

#include "media.h"

#include "adts.h"

/* What the ADTS headers of raw AAC need, from its AudioSpecificConfig:
 * object type (5 bits), rate index (4), channel configuration (4). SBR/PS
 * (HE-AAC) signal the core. -1 without a configuration. */
static int read_aac_config(mana_media *m, const AVCodecParameters *audio) {
    const uint8_t *c = audio->extradata;
    if (audio->extradata_size < 2) return -1;
    int object_type = c[0] >> 3, rate_index = ((c[0] & 7) << 1) | (c[1] >> 7);
    m->aac_object_type = object_type == 5 || object_type == 29 ? 2 : object_type;
    m->aac_rate_index = rate_index < 12 ? rate_index : aac_rate_index(audio->sample_rate);
    m->aac_channels = (c[1] >> 3) & 0x0F;
    return 0;
}

/* MP4 carries length-prefixed NAL units and raw AAC; the TV takes Annex B
 * and ADTS, as in MPEG-TS. */
int setup_mp4(mana_media *m, mana_error *err) {
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
            fail(err, MANA_STEP_ANNEXB, r < 0 ? r : AVERROR(ENOMEM));
            return -1;
        }
    }
    if (m->audio_index >= 0) {
        AVCodecParameters *audio = m->fmt->streams[m->audio_index]->codecpar;
        if (audio->codec_id == AV_CODEC_ID_AAC && read_aac_config(m, audio) < 0) {
            fail(err, MANA_STEP_AAC_CONFIG, 0);
            return -1;
        }
    }
    return 0;
}
