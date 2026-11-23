/* Reads ahead for the video size when the stream info leaves it out, and
 * keeps the packets read for playback (media.h). */

#include "media.h"

void clear_queue(mana_media *m) {
    while (m->queue_count) {
        av_packet_free(&m->queue[m->queue_head]);
        m->queue_head = (m->queue_head + 1) % PROBE_PACKETS;
        m->queue_count--;
    }
    m->queue_head = 0;
}

/* Next demuxed packet: queued probe packets first, then the demuxer. */
int next_packet(mana_media *m, AVPacket *out) {
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
void probe_video_size(mana_media *m, mana_info *info) {
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
