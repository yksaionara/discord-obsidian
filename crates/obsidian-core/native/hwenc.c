#include <stdlib.h>
/* Hardware encoding for Go Live, through FFmpeg's encoders in
 * vendor/ffmpeg-src: NVIDIA's NVENC, then AMD's AMF on Windows and VA-API
 * (AMD and Intel) on Linux, then on Windows the hardware encoder Media
 * Foundation lists, which every GPU maker's driver provides: Intel's Quick
 * Sync, and any GPU the others don't open on.
 * The same interface as h264.c, so Rust drives both alike: I420 frames in,
 * Annex B access units out, each NAL unit after a start code. Opening tries
 * each encoder in turn and says which one opened, or why none did.
 *
 * Settings, as measured on screen recordings and games
 * (docs/discord/go-live.md):
 * low-latency tuning, no B-frames, a frame out for each frame in, the
 * bitrate capped over an eighth of a second without padding a calm screen,
 * H264's Main profile, and parameter sets before each IDR. */
#define _POSIX_C_SOURCE 200809L
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#ifdef __linux__
#include <unistd.h>
#endif

#include "libavcodec/avcodec.h"
#include "libavutil/hwcontext.h"
#include "libavutil/opt.h"
#include "libavutil/time.h"

/* The codecs, as Rust numbers them. */
enum { CODEC_H264, CODEC_HEVC, CODEC_AV1 };

struct obsidian_hwenc {
  AVCodecContext *codec;
  /* VA-API's device, and the frame its encoder reads, on the GPU. */
  AVBufferRef *device;
  AVFrame *surface;
  /* The frame handed over, in memory: I420 for NVENC, NV12 for VA-API. */
  AVFrame *frame;
  AVPacket *packet;
  int64_t index;
  /* AMF hands a frame back a little after taking it, and is polled for it. */
  int poll;
};

/* FFmpeg's first error on this thread since it was cleared, for saying why
 * an encoder didn't open: the cause, where later ones only say what failed
 * because of it. */
static _Thread_local char first_error[160];

static void keep_error(void *context, int level, const char *format, va_list args) {
  (void)context;
  if (level > AV_LOG_ERROR || first_error[0]) return;
  vsnprintf(first_error, sizeof first_error, format, args);
  size_t length = strlen(first_error);
  if (length && first_error[length - 1] == '\n') first_error[length - 1] = 0;
}

static void set_rate(AVCodecContext *codec, unsigned kbps) {
  codec->bit_rate = (int64_t)kbps * 1000;
  codec->rc_max_rate = (int64_t)kbps * 1000;
  /* An eighth of a second of bits, as realtime streaming keeps it: a
   * second's let one keyframe after a calm moment reach a megabyte at
   * 10 Mbit/s, which a home upload dropped most of, freezing viewers. A
   * keyframe starts a little soft and sharpens over the next frames. */
  codec->rc_buffer_size = (int)(kbps * 1000 / 8);
}

static void close_encoder(struct obsidian_hwenc *h) {
  avcodec_free_context(&h->codec);
  av_frame_free(&h->surface);
  av_frame_free(&h->frame);
  av_packet_free(&h->packet);
  av_buffer_unref(&h->device);
}

/* A render node for VA-API, and the libva driver for it. */
struct va_device {
  const char *path, *driver;
};

/* VA-API's device and a pool of NV12 surfaces for the encoder to read. */
static int open_vaapi(struct obsidian_hwenc *h, AVCodecContext *codec, struct va_device device) {
  AVDictionary *options = NULL;
  av_dict_set(&options, "driver", device.driver, 0);
  int ret = av_hwdevice_ctx_create(&h->device, AV_HWDEVICE_TYPE_VAAPI, device.path, options, 0);
  av_dict_free(&options);
  if (ret < 0) return ret;
  AVBufferRef *frames = av_hwframe_ctx_alloc(h->device);
  if (!frames) return AVERROR(ENOMEM);
  AVHWFramesContext *pool = (AVHWFramesContext *)frames->data;
  pool->format = AV_PIX_FMT_VAAPI;
  pool->sw_format = AV_PIX_FMT_NV12;
  pool->width = codec->width;
  pool->height = codec->height;
  pool->initial_pool_size = 2;
  ret = av_hwframe_ctx_init(frames);
  if (ret >= 0) codec->hw_frames_ctx = av_buffer_ref(frames);
  av_buffer_unref(&frames);
  if (ret < 0) return ret;
  h->surface = av_frame_alloc();
  return h->surface ? 0 : AVERROR(ENOMEM);
}

static int open_encoder(struct obsidian_hwenc *h, const AVCodec *found, struct va_device device,
                        unsigned codec_id, unsigned width, unsigned height, unsigned fps,
                        unsigned kbps) {
  int vaapi = device.path != NULL, amf = strstr(found->name, "_amf") != NULL,
      mf = strstr(found->name, "_mf") != NULL;
  AVCodecContext *codec = h->codec = avcodec_alloc_context3(found);
  if (!codec) return AVERROR(ENOMEM);
  codec->width = (int)width;
  codec->height = (int)height;
  codec->time_base = (AVRational){1, (int)fps};
  codec->framerate = (AVRational){(int)fps, 1};
  /* Keyframes when a viewer asks, as Discord's client sends them, not on a
   * timer: each is the largest frame there is, and one every 3 seconds froze
   * viewers on a busy upload. A new viewer asks for one. This only bounds
   * how long a stream goes without one, at a minute. */
  codec->gop_size = (int)fps * 60;
  codec->max_b_frames = 0;
  codec->pix_fmt = vaapi ? AV_PIX_FMT_VAAPI : amf || mf ? AV_PIX_FMT_NV12 : AV_PIX_FMT_YUV420P;
  /* libyuv converts captured pixels with BT.601's limited range. */
  codec->color_primaries = AVCOL_PRI_SMPTE170M;
  codec->color_trc = AVCOL_TRC_SMPTE170M;
  codec->colorspace = AVCOL_SPC_SMPTE170M;
  codec->color_range = AVCOL_RANGE_MPEG;
  set_rate(codec, kbps);
  if (codec_id == CODEC_H264) codec->profile = AV_PROFILE_H264_MAIN;
  void *options = codec->priv_data;
  int ret = 0;
  if (vaapi) {
    av_opt_set(options, "rc_mode", "VBR", 0);
    av_opt_set_int(options, "async_depth", 1, 0);
    ret = open_vaapi(h, codec, device);
    if (ret < 0) return ret;
  } else if (amf) {
    /* Or FFmpeg holds each frame's packet back until the next frame. */
    codec->flags |= AV_CODEC_FLAG_LOW_DELAY;
    av_opt_set(options, "usage", "lowlatency", 0);
    av_opt_set(options, "quality", "balanced", 0);
    av_opt_set(options, "rc", "vbr_peak", 0);
    av_opt_set_int(options, "enforce_hrd", 1, 0);
    av_opt_set_int(options, "filler_data", 0, 0);
    av_opt_set_int(options, "async_depth", 1, 0);
    av_opt_set_int(options, "forced_idr", 1, 0);
    av_opt_set_int(options, "aud", 0, 0);
    if (codec_id == CODEC_AV1) {
      av_opt_set(options, "latency", "real_time", 0);
      av_opt_set(options, "header_insertion_mode", "frame", 0);
      av_opt_set_int(options, "bf", 0, 0);
    } else {
      av_opt_set_int(options, "latency", 1, 0);
    }
    if (codec_id == CODEC_HEVC) av_opt_set(options, "header_insertion_mode", "idr", 0);
    if (codec_id == CODEC_H264) {
      av_opt_set(options, "profile", "main", 0);
      av_opt_set_int(options, "bf", 0, 0);
      av_opt_set_int(options, "header_spacing", codec->gop_size, 0);
    }
    h->poll = 1;
  } else if (mf) {
    /* Only a GPU's encoder: Microsoft's own runs on the processor, where
     * x264 does better. FFmpeg waits for the encoder's events, so its
     * packet comes as soon as it's made, without polling. A setting the
     * driver doesn't have is left at its default. */
    codec->flags |= AV_CODEC_FLAG_LOW_DELAY;
    av_opt_set_int(options, "hw_encoding", 1, 0);
    av_opt_set(options, "scenario", "display_remoting", 0);
    av_opt_set(options, "rate_control", "pc_vbr", 0);
  } else {
    /* p6 with a quarter-size first pass up to 60 fps (6 ms a 1440p AV1
       frame on an RTX 5070 Ti, of the 16 it has); p5 up to 120; p4, Discord's own quality, up to 240; p3 above, so a frame still
       encodes within its 2.8 ms at 360. On an RTX 5070 Ti at 1568x1080, AV1
       takes 1.9 ms a frame at p4 and 1.8 at p3 (p1, 1.4, blurred text).
       OBSIDIAN_NVENC_PRESET overrides it, for measuring. */
    const char *preset = getenv("OBSIDIAN_NVENC_PRESET");
    av_opt_set(options, "preset", preset ? preset : fps > 240 ? "p3" : fps > 120 ? "p4" : fps > 60 ? "p5" : "p6", 0);
    av_opt_set(options, "tune", "ll", 0);
    av_opt_set(options, "rc", "vbr", 0);
    if (fps <= 60) av_opt_set(options, "multipass", "qres", 0);
    /* More bits where the eye sees blocks: flat skies, dark corners. */
    av_opt_set_int(options, "spatial-aq", 1, 0);
    av_opt_set_int(options, "aq-strength", 8, 0);
    av_opt_set_int(options, "zerolatency", 1, 0);
    av_opt_set_int(options, "delay", 0, 0);
    av_opt_set_int(options, "forced-idr", 1, 0);
    if (codec_id == CODEC_H264) av_opt_set(options, "profile", "main", 0);
  }
  ret = avcodec_open2(codec, found, NULL);
  if (ret < 0) return ret;
  h->frame = av_frame_alloc();
  h->packet = av_packet_alloc();
  if (!h->frame || !h->packet) return AVERROR(ENOMEM);
  h->frame->format = vaapi || amf || mf ? AV_PIX_FMT_NV12 : AV_PIX_FMT_YUV420P;
  h->frame->width = (int)width;
  h->frame->height = (int)height;
  return av_frame_get_buffer(h->frame, 0);
}

/* The encoders to try for each codec, best first; each system builds in
 * those it has. */
static const char *const ENCODERS[][5] = {
    [CODEC_H264] = {"h264_nvenc", "h264_amf", "h264_vaapi", "h264_mf", NULL},
    [CODEC_HEVC] = {"hevc_nvenc", "hevc_amf", "hevc_vaapi", "hevc_mf", NULL},
    [CODEC_AV1] = {"av1_nvenc", "av1_amf", "av1_vaapi", "av1_mf", NULL},
};

/* The libva drivers for each kernel driver whose GPUs encode, best first.
 * Naming the driver overrides LIBVA_DRIVER_NAME, which NixOS and others
 * set to nvidia for browsers, and which would fail every other GPU. NVIDIA's
 * own VA-API driver only decodes; NVENC encodes there. */
static const char *const VA_DRIVERS[][4] = {
    {"amdgpu", "radeonsi", NULL},
    {"radeon", "r600", NULL},
    {"i915", "iHD", "i965"},
    {"xe", "iHD", NULL},
    {"virtio_gpu", "virtio_gpu", NULL},
};

/* The kernel driver of /dev/dri/renderD<128 + n>, or an empty string. */
static void kernel_driver(int n, char *name, size_t size) {
  name[0] = 0;
#ifdef __linux__
  char link[64], target[256];
  snprintf(link, sizeof link, "/sys/class/drm/renderD%d/device/driver", 128 + n);
  ssize_t length = readlink(link, target, sizeof target - 1);
  if (length <= 0) return;
  target[length] = 0;
  const char *base = strrchr(target, '/');
  base = base ? base + 1 : target;
  if (strlen(base) < size) memcpy(name, base, strlen(base) + 1);
#else
  (void)n;
  (void)size;
#endif
}

struct opening {
  unsigned codec_id, width, height, fps, kbps;
  char *report;
  size_t report_size, used;
};

/* Opens `found` on `device`, or adds why it didn't open to the report. */
static struct obsidian_hwenc *try_open(struct opening *o, const AVCodec *found,
                                      struct va_device device) {
  const char *name = found->name;
  char label[96];
  if (device.path)
    snprintf(label, sizeof label, "%s (%s, %s)", name, device.path + strlen("/dev/dri/"),
             device.driver);
  else
    snprintf(label, sizeof label, "%s", name);
  struct obsidian_hwenc *h = av_mallocz(sizeof *h);
  if (!h) return NULL;
  first_error[0] = 0;
  int ret = open_encoder(h, found, device, o->codec_id, o->width, o->height, o->fps, o->kbps);
  if (ret >= 0) {
    snprintf(o->report, o->report_size, "%s", label);
    return h;
  }
  if (!first_error[0]) av_strerror(ret, first_error, sizeof first_error);
  if (o->used < o->report_size)
    o->used += (size_t)snprintf(o->report + o->used, o->report_size - o->used, "%s%s: %s",
                                o->used ? "; " : "", label, first_error);
  close_encoder(h);
  av_free(h);
  return NULL;
}

/* Opens the first encoder of `codec_id` that works, VA-API's on each GPU
 * that has a driver for it, and writes its name to `report`, or each one's
 * failure when none does. */
struct obsidian_hwenc *obsidian_hwenc_new(unsigned codec_id, unsigned width, unsigned height,
                                        unsigned fps, unsigned kbps, char *report,
                                        size_t report_size) {
  if (report_size) report[0] = 0;
  if (codec_id > CODEC_AV1) return NULL;
  av_log_set_level(AV_LOG_ERROR);
  av_log_set_callback(keep_error);
  struct opening o = {codec_id, width, height, fps, kbps, report, report_size, 0};
  /* OBSIDIAN_HWENC=_mf tries only the encoders with that in their name, to
   * try one where another would open first, as Media Foundation's behind
   * NVENC on an NVIDIA GPU. */
  const char *only = getenv("OBSIDIAN_HWENC");
  for (const char *const *name = ENCODERS[codec_id]; *name; name++) {
    if (only && *only && !strstr(*name, only)) continue;
    const AVCodec *found = avcodec_find_encoder_by_name(*name);
    if (!found) continue;
    struct obsidian_hwenc *h = NULL;
    if (!strstr(*name, "_vaapi")) {
      h = try_open(&o, found, (struct va_device){NULL, NULL});
      if (h) return h;
      continue;
    }
    for (int n = 0; n < 8; n++) {
      char kernel[32], path[32];
      kernel_driver(n, kernel, sizeof kernel);
      snprintf(path, sizeof path, "/dev/dri/renderD%d", 128 + n);
      for (size_t i = 0; i < sizeof VA_DRIVERS / sizeof *VA_DRIVERS; i++) {
        if (strcmp(kernel, VA_DRIVERS[i][0])) continue;
        for (const char *const *driver = VA_DRIVERS[i] + 1; *driver; driver++) {
          h = try_open(&o, found, (struct va_device){path, *driver});
          if (h) return h;
        }
      }
    }
  }
  return NULL;
}

/* Copies an I420 picture into the frame the encoder reads: as it is for
 * NVENC, with its chroma planes interleaved for VA-API's and AMF's NV12. */
static int fill(struct obsidian_hwenc *h, const uint8_t *y, int y_stride, const uint8_t *u,
                int u_stride, const uint8_t *v, int v_stride) {
  AVFrame *frame = h->frame;
  int ret = av_frame_make_writable(frame);
  if (ret < 0) return ret;
  int width = frame->width, height = frame->height;
  for (int row = 0; row < height; row++)
    memcpy(frame->data[0] + row * frame->linesize[0], y + row * y_stride, (size_t)width);
  for (int row = 0; row < height / 2; row++) {
    const uint8_t *u_row = u + row * u_stride, *v_row = v + row * v_stride;
    if (frame->format == AV_PIX_FMT_NV12) {
      uint8_t *uv = frame->data[1] + row * frame->linesize[1];
      for (int x = 0; x < width / 2; x++) {
        uv[2 * x] = u_row[x];
        uv[2 * x + 1] = v_row[x];
      }
    } else {
      memcpy(frame->data[1] + row * frame->linesize[1], u_row, (size_t)width / 2);
      memcpy(frame->data[2] + row * frame->linesize[2], v_row, (size_t)width / 2);
    }
  }
  return 0;
}

/* Whether the Annex B unit at `unit` is an access unit delimiter in
 * `codec`'s numbering: H264's type 9, HEVC's 35. */
static int delimiter(enum AVCodecID codec, const uint8_t *unit, size_t size) {
  if (!size) return 0;
  if (codec == AV_CODEC_ID_H264) return (unit[0] & 0x1f) == 9;
  if (codec == AV_CODEC_ID_HEVC) return ((unit[0] >> 1) & 0x3f) == 35;
  return 0;
}

static const uint8_t START_CODE[4] = {0, 0, 0, 1};

/* Appends `data`'s units to `out` at `*size`, each after a four-byte start
 * code, leaving out access unit delimiters: Media Foundation's encoders
 * start each frame with one, which no other encoder here sends, nor
 * Discord's client. Returns -2 when `out` is too small. AV1 goes as it is. */
static int append_units(enum AVCodecID codec, const uint8_t *data, size_t length, uint8_t *out,
                        size_t capacity, size_t *size) {
  if (codec != AV_CODEC_ID_H264 && codec != AV_CODEC_ID_HEVC) {
    if (*size + length > capacity) return -2;
    memcpy(out + *size, data, length);
    *size += length;
    return 0;
  }
  size_t at = 0;
  /* Each unit's first byte, past its start code. */
  while (at + 3 <= length) {
    if (data[at] == 0 && data[at + 1] == 0 && data[at + 2] == 1) {
      at += 3;
      break;
    }
    at++;
  }
  while (at < length) {
    size_t end = at, next = length;
    while (end + 3 <= length) {
      if (data[end] == 0 && data[end + 1] == 0 && (data[end + 2] == 1 ||
          (end + 4 <= length && data[end + 2] == 0 && data[end + 3] == 1))) {
        next = end + (data[end + 2] == 1 ? 3 : 4);
        break;
      }
      end++;
    }
    if (end + 3 > length) end = length;
    if (!delimiter(codec, data + at, end - at)) {
      if (*size + sizeof START_CODE + (end - at) > capacity) return -2;
      memcpy(out + *size, START_CODE, sizeof START_CODE);
      memcpy(out + *size + sizeof START_CODE, data + at, end - at);
      *size += sizeof START_CODE + (end - at);
    }
    at = next;
  }
  return 0;
}

/* Encodes one I420 frame into `out`. Returns the encoded size, 0 when the
 * encoder gave no frame, or a negative number on failure. `pts` and
 * `duration` are unused: frames are timed by the frame rate. */
long obsidian_hwenc_encode(struct obsidian_hwenc *h, const unsigned char *y, int y_stride,
                          const unsigned char *u, int u_stride, const unsigned char *v,
                          int v_stride, long long pts, unsigned long duration, int keyframe,
                          unsigned char *out, size_t capacity, int *is_key) {
  (void)pts;
  (void)duration;
  int ret = fill(h, y, y_stride, u, u_stride, v, v_stride);
  if (ret < 0) return -1;
  AVFrame *frame = h->frame;
  if (h->surface) {
    av_frame_unref(h->surface);
    ret = av_hwframe_get_buffer(h->codec->hw_frames_ctx, h->surface, 0);
    if (ret >= 0) ret = av_hwframe_transfer_data(h->surface, h->frame, 0);
    if (ret < 0) return -1;
    frame = h->surface;
  }
  frame->pts = h->index++;
  frame->pict_type = keyframe ? AV_PICTURE_TYPE_I : AV_PICTURE_TYPE_NONE;
  if (avcodec_send_frame(h->codec, frame) < 0) return -1;
  size_t size = 0;
  *is_key = 0;
  /* AMF waits a millisecond at most for each receive: polled for the
   * frame's packet for up to 100 ms, after which it comes with the next. */
  int64_t deadline = h->poll ? av_gettime_relative() + 100000 : 0;
  for (;;) {
    ret = avcodec_receive_packet(h->codec, h->packet);
    if (ret == AVERROR(EAGAIN) && !size && av_gettime_relative() < deadline) {
      /* Older drivers don't wait in AMF's receive at all. */
      av_usleep(250);
      continue;
    }
    if (ret < 0) break;
    if (append_units(h->codec->codec_id, h->packet->data, (size_t)h->packet->size, out, capacity,
                     &size) < 0) {
      av_packet_unref(h->packet);
      return -2;
    }
    *is_key |= (h->packet->flags & AV_PKT_FLAG_KEY) != 0;
    av_packet_unref(h->packet);
  }
  if (ret != AVERROR(EAGAIN)) return -1;
  return (long)size;
}

/* Changes the bitrate aimed for, from the next frame on, without a
 * keyframe: patches/ in vendor/ffmpeg-src make NVENC, VA-API, AMF and Media
 * Foundation follow it.
 * Returns 0. */
int obsidian_hwenc_set_bitrate(struct obsidian_hwenc *h, unsigned kbps) {
  set_rate(h->codec, kbps);
  return 0;
}

void obsidian_hwenc_free(struct obsidian_hwenc *h) {
  close_encoder(h);
  av_free(h);
}
