#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include <libavcodec/avcodec.h>
#include <libavutil/channel_layout.h>
#include <libavutil/error.h>
#include <libavutil/frame.h>
#include <libavutil/mem.h>
#include <libavutil/samplefmt.h>
#include <libswresample/swresample.h>

#include "error.h"
#include "log.h"

// Streams the desktop audio as raw AAC-LC wrapped in ADTS headers. The
// browser plays this with a plain <audio> element via the /audio_stream
// HTTP route. Only libavcodec + libswresample are used; no muxer is
// needed since the ADTS header for 48kHz stereo AAC-LC is constant apart
// from the frame length.
#define AUDIO_OUT_RATE 48000

typedef struct AudioContext
{
	AVCodecContext* c;
	SwrContext* swr;
	AVFrame* frame;
	AVFrame* in_frame;
	AVPacket* pkt;
	void* rust_ctx;

	// resampled pcm fifo, planar, out layout is always stereo
	float* fifo_l;
	float* fifo_r;
	int fifo_len;
	int fifo_cap;

	int frame_size;
	int in_rate;
	int in_channels;
	int64_t pts;
	int initialized;
} AudioContext;

// this is a rust function and lives in src/audio.rs
int write_audio_packet(void* rust_ctx, const uint8_t* buf, int buf_size);

// frame length includes the 7 byte ADTS header, out is always 48kHz stereo
// AAC-LC so the fields other than the length are constant.
static void adts_header(uint8_t* hdr, int frame_length)
{
	hdr[0] = 0xFF;
	hdr[1] = 0xF1; // MPEG-4, layer 0, no CRC
	hdr[2] = 0x4C; // profile AAC-LC (1), sampling frequency index 3 (48kHz), private 0, channel config hi
	hdr[3] = 0x80 | ((frame_length >> 11) & 0x03); // channel config lo, rest of the flags
	hdr[4] = (frame_length >> 3) & 0xFF;
	hdr[5] = ((frame_length & 0x07) << 5) | 0x1F; // buffer fullness max
	hdr[6] = 0xFC; // buffer fullness max, one frame per block
}

void init_audio_sink(AudioContext* ctx, int sample_rate_in, int channels_in, Error* err)
{
	const AVCodec* codec;
	int ret;

	codec = avcodec_find_encoder(AV_CODEC_ID_AAC);
	if (!codec)
		ERROR(err, 1, "Codec 'aac' not found!");

	ctx->c = avcodec_alloc_context3(codec);
	if (!ctx->c)
		ERROR(err, 1, "Could not allocate audio codec context");

	ctx->c->sample_fmt = AV_SAMPLE_FMT_FLTP;
	ctx->c->sample_rate = AUDIO_OUT_RATE;
	ctx->c->bit_rate = 192000;

	av_channel_layout_default(&ctx->c->ch_layout, 2);

	if (avcodec_open2(ctx->c, codec, NULL) < 0)
		ERROR(err, 1, "Could not open audio codec 'aac'!");

	ctx->frame_size = ctx->c->frame_size > 0 ? ctx->c->frame_size : 1024;

	ctx->frame = av_frame_alloc();
	if (!ctx->frame)
		ERROR(err, 1, "Failed to allocate audio frame!");

	ctx->frame->format = AV_SAMPLE_FMT_FLTP;
	ctx->frame->sample_rate = AUDIO_OUT_RATE;
	ctx->frame->nb_samples = ctx->frame_size;
	av_channel_layout_copy(&ctx->frame->ch_layout, &ctx->c->ch_layout);
	if (av_frame_get_buffer(ctx->frame, 0) < 0)
		ERROR(err, 1, "Failed to allocate buffer for audio frame!");

	AVChannelLayout in_layout;
	av_channel_layout_default(&in_layout, channels_in);
	ret = swr_alloc_set_opts2(
		&ctx->swr,
		&ctx->c->ch_layout,
		AV_SAMPLE_FMT_FLTP,
		AUDIO_OUT_RATE,
		&in_layout,
		AV_SAMPLE_FMT_FLT,
		sample_rate_in,
		0,
		NULL);
	if (ret < 0 || swr_init(ctx->swr) < 0)
		ERROR(err, 1, "Failed to create resampling context (in: %d Hz, %d channel(s))!", sample_rate_in, channels_in);

	// fifo large enough for the resampled output of any chunk plus the
	// codec frame size
	ctx->fifo_cap = (int)((int64_t)ctx->frame_size * 4 + 8192);
	ctx->fifo_l = av_malloc(sizeof(float) * ctx->fifo_cap);
	ctx->fifo_r = av_malloc(sizeof(float) * ctx->fifo_cap);
	if (!ctx->fifo_l || !ctx->fifo_r)
		ERROR(err, 1, "Failed to allocate audio fifo!");
	ctx->fifo_len = 0;

	ctx->pkt = av_packet_alloc();
	if (!ctx->pkt)
		ERROR(err, 1, "Failed to allocate audio packet!");

	ctx->in_rate = sample_rate_in;
	ctx->in_channels = channels_in;
	ctx->initialized = 1;

	log_info(
		"Audio: %d Hz, %d channel(s) -> %d Hz stereo @%s (ADTS)",
		sample_rate_in,
		channels_in,
		AUDIO_OUT_RATE,
		ctx->c->codec->name);
}

void encode_audio_slice(AudioContext* ctx, Error* err)
{
	int ret;

	while (ctx->fifo_len >= ctx->frame_size)
	{
		// copy planar fifo slice into out frame
		memcpy(
			ctx->frame->data[0],
			ctx->fifo_l,
			sizeof(float) * ctx->frame_size);
		memcpy(
			ctx->frame->data[1],
			ctx->fifo_r,
			sizeof(float) * ctx->frame_size);
		memmove(
			ctx->fifo_l,
			ctx->fifo_l + ctx->frame_size,
			sizeof(float) * (ctx->fifo_len - ctx->frame_size));
		memmove(
			ctx->fifo_r,
			ctx->fifo_r + ctx->frame_size,
			sizeof(float) * (ctx->fifo_len - ctx->frame_size));
		ctx->fifo_len -= ctx->frame_size;

		ctx->frame->pts = ctx->pts;
		ctx->pts += ctx->frame_size;

		ret = avcodec_send_frame(ctx->c, ctx->frame);
		if (ret < 0)
			ERROR(err, ret, "Error sending an audio frame for encoding: %s!", av_err2str(ret));

		while (1)
		{
			ret = avcodec_receive_packet(ctx->c, ctx->pkt);
			if (ret == AVERROR(EAGAIN) || ret == AVERROR_EOF)
				break;
			if (ret < 0)
				ERROR(err, ret, "Error while encoding audio packet: %s!", av_err2str(ret));

			// ADTS wrapped packet for the browser <audio> element
			uint8_t packet_buf[7 + 2048 * 2];
			if (ctx->pkt->size > 2048 * 2)
			{
				log_warn("Audio packet too large for ADTS buffer: %d bytes", ctx->pkt->size);
				av_packet_unref(ctx->pkt);
				continue;
			}
			adts_header(packet_buf, 7 + ctx->pkt->size);
			memcpy(packet_buf + 7, ctx->pkt->data, ctx->pkt->size);
			write_audio_packet(ctx->rust_ctx, packet_buf, 7 + ctx->pkt->size);
			av_packet_unref(ctx->pkt);
		}
	}
}

void encode_audio_chunk(AudioContext* ctx, const void* samples, int frames, Error* err)
{
	int out_frames;

	if (frames > 0)
	{
		float* out_l = av_malloc(sizeof(float) * (frames * 4 + 64));
		float* out_r = av_malloc(sizeof(float) * (frames * 4 + 64));
		if (!out_l || !out_r)
		{
			av_free(out_l);
			av_free(out_r);
			ERROR(err, 1, "Failed to allocate audio resample buffers!");
		}

		uint8_t* out_slices[2] = {
			(uint8_t*)out_l,
			(uint8_t*)out_r
		};
		const uint8_t* in_ptr = samples;
		out_frames = swr_convert(
			ctx->swr,
			out_slices,
			frames * 4 + 64,
			&in_ptr,
			frames);
		if (out_frames < 0)
		{
			av_free(out_l);
			av_free(out_r);
			ERROR(err, out_frames, "Error resampling audio: %s!", av_err2str(out_frames));
		}

		// append converted samples to the planar fifo
		if (ctx->fifo_len + out_frames > ctx->fifo_cap)
		{
			// this should not happen, but never write past the fifo
			log_warn("Audio fifo overflow, dropping %d frame(s)!", ctx->fifo_len + out_frames - ctx->fifo_cap);
			out_frames = ctx->fifo_cap - ctx->fifo_len;
		}
		memcpy(ctx->fifo_l + ctx->fifo_len, out_l, sizeof(float) * out_frames);
		memcpy(ctx->fifo_r + ctx->fifo_len, out_r, sizeof(float) * out_frames);
		ctx->fifo_len += out_frames;

		av_free(out_l);
		av_free(out_r);
	}

	encode_audio_slice(ctx, err);
	OK_OR_ABORT(err)

	// Only when the fifo is completely empty, keep the stream alive by
	// encoding one frame of silence: a partial frame combined with a slow
	// input would otherwise be zero-padded on every call, making the
	// playback sound choppy.
	if (ctx->fifo_len == 0)
	{
		memset(ctx->fifo_l, 0, sizeof(float) * ctx->frame_size);
		memset(ctx->fifo_r, 0, sizeof(float) * ctx->frame_size);
		ctx->fifo_len = ctx->frame_size;
		encode_audio_slice(ctx, err);
		OK_OR_ABORT(err)
	}
}

int get_audio_frame_size(AudioContext* ctx)
{
	return ctx->frame_size;
}

void destroy_audio_encoder(AudioContext* ctx)
{
	if (ctx->initialized)
	{
		swr_free(&ctx->swr);
		av_frame_free(&ctx->frame);
		av_packet_free(&ctx->pkt);
		av_free(ctx->fifo_l);
		av_free(ctx->fifo_r);
	}
	if (ctx->c)
		avcodec_free_context(&ctx->c);
	free(ctx);
}

AudioContext* init_audio_encoder(void* rust_ctx)
{
	AudioContext* ctx = malloc(sizeof(AudioContext));
	ctx->rust_ctx = rust_ctx;
	ctx->c = NULL;
	ctx->swr = NULL;
	ctx->frame = NULL;
	ctx->pkt = NULL;
	ctx->fifo_l = NULL;
	ctx->fifo_r = NULL;
	ctx->fifo_len = 0;
	ctx->fifo_cap = 0;
	ctx->frame_size = 0;
	ctx->in_rate = 0;
	ctx->in_channels = 0;
	ctx->pts = 0;
	ctx->initialized = 0;
	return ctx;
}
