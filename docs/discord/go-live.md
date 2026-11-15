# Go Live

Sharing a screen or window with its sound on Discord, and watching others'
streams. The rest of Discord is in [the Discord docs](README.md).

Go Live streams come through Discord's gateway: op 18 starts sharing, op 20
watches someone, op 22 pauses and op 19 ends either. Discord answers with
STREAM_CREATE, naming the RTC server, then STREAM_SERVER_UPDATE with an
endpoint and token. The stream gets a voice connection of its own to that
server, reusing the call's voice session id; a stream waits for that id too,
if this account's voice state arrives after both answers. `obsidian_discord::stream` names
streams by key (`guild:{server}:{channel}:{user}`) and
`Discord::{go_live, watch, end_stream, pause_stream}` run the signalling. Voice
states carry `self_stream`, which shows as a LIVE badge beside the person.
Sharing starts from the call's controls or from the voice panel's Share
button, which works wherever the window is; the panel says when the stream
is live and stops it. A click on someone's tile watches their stream, which
then shows large in it; someone who films as well has a second tile for the
stream. A stream being watched starts large, however watching began, and
stays watched in its tile after a click beside it. While the call is off
screen, the large tile's stream or camera, or else the stream being watched,
or this account's own, shows in a corner (`StreamPip`), as Discord's
picture in picture does. STREAM_CREATE and STREAM_UPDATE name a stream's
viewers in `viewer_ids`; Discord Obsidian keeps them for the stream it shares and the one
it watches, and shows them over the stream (`ViewerStack`) and as a count in
the voice panel; a click on it returns to the call. The Who's Watching
add-on also lists the stream's viewers in the voice panel, by avatar
([the chat UI doc](../chat-ui.md)). This account's
own stream has a tile of its own, which opens large as others' do, without
watching it through Discord: it shows the worker's preview.

Capture runs in a worker process, `obsidian --stream-worker`, so its buffers take
memory only while sharing; the process exits when sharing stops. Playing a
video file runs there too (`Job::Play`, `video_player`), which also keeps
FFmpeg's reading of files people sent out of the window's process. Its
frames carry their time, which the seek bar shows.
`stream_worker` starts it with one job as a JSON line and reads its reports
back. On Linux it starts `/proc/self/exe`, the running Discord Obsidian, since Discord Obsidian's path
names a deleted file once a rebuild or update replaces it. Frames reach the UI through `frames`, a triple buffer in a memory-mapped
file in the runtime directory. The file is unlinked once the worker opens it.
Windows won't delete a mapped file, so there it's a temporary file that
Windows deletes once both processes close it.
On Linux, libwebrtc's capturer asks the screen sharing portal what to share.
The portal answers over D-Bus through GLib, so the worker runs GLib's main
context while it waits for the dialog and for frames. Discord Obsidian tells Discord it's
live only once frames flow, so a cancelled dialog shares nothing. Windows has
no such dialog, so Discord Obsidian's share dialog, laid out as Discord's, lists the
screens and windows libwebrtc's capturers find (`stream_worker::shareable`)
under Applications and Entire Screen tabs, and cameras under Devices, and the
worker captures the one picked by its id. libwebrtc leaves out minimized
windows, and a game in full screen minimizes as soon as the dialog takes the
focus, so Valorant was never there to pick: on Windows the windows the
taskbar would show come after libwebrtc's (`top_windows`: visible, minimized
or not, titled, not tool windows or owned, not cloaked, not Discord
Obsidian's), and the window of the game Discord sees being played comes
first, matched by its title. A minimized window gets no picture, only its
icon. Window capture opens on a minimized window and gives frames once it's
back; see the
[Windows guide](../windows-beta.md#go-live-on-windows). A glance worker
(`Job::Glance`) pictures each screen and window, 448 by 252 points, and each
window's app icon comes from the window itself (`src/window_icon.rs`); the
pictures go when the dialog closes. A click on one goes live at once, at the
quality its foot shows, which the gear changes: presets, resolution, frame
rate, the encoder's effort, and whether other apps' sound goes too.
While live, the share button opens Discord's menu instead: Stop Streaming,
Change Stream, Stream Quality, Share Stream Audio and Report Problem. Stream
Quality and Share Stream Audio change the stream as it goes
(`Control::Quality`, `Control::Sound`): the worker captures the same source
again at the new size, its encoder opens again at the new rate, and the
stream's sound capture stops or starts, all on the same connection.
Change Stream switches what's shared without
ending the stream: the worker starts a second capturer (on Linux, through the
dialog again) and keeps sending the first one's frames. When the second one
gives a frame, it replaces the first. If the dialog is cancelled, the first
one goes on. The encoder takes the new size on the same connection.
`OBSIDIAN_STREAM_TEST_PATTERN=1` shares moving colour bars instead of a screen.

A camera can be shared too, such as OBS's virtual camera (v4l2loopback on
Linux, its DirectShow filter on Windows). The share dialog lists the cameras
[the call's camera](voice.md#camera-video) lists, under Devices, counted after
the screens and windows; on Linux, where the portal picks a screen or window, a
"Screen or window" choice comes first, and switching with no camera listed
goes straight to the dialog. The worker films the camera through the same
capture module (`camera::Camera::open_at`), asking for the stream's
resolution and frame rate rather than the call camera's 720p at 30, and
copies each picture into one buffer it keeps for the next before the encoder
takes it. Switching between a camera and a screen works as between two
screens. `OBSIDIAN_CAMERA_TEST_PATTERN=1` lists a test pattern camera to share.

Once Discord has started the stream, Discord Obsidian unpauses it with op 22, as
Discord-video-stream does, and the worker gets its server and sends the video
there (`obsidian_discord::stream_connection`). The connection is a call's
(`voice_connection.rs`), on voice gateway version 8 as Discord's clients use:
heartbeats acknowledge the last sequence number, and binary DAVE messages start
with one. It identifies with the stream's RTC server and `video: true`,
discovers its address, then uses `aead_aes256_gcm_rtpsize` transport
encryption. Discord Obsidian offers H264, and AV1 then H265 before it where the GPU
encodes them ([Encoding on the GPU](#encoding-on-the-gpu)), and announces its
size with op 12, again whenever someone connects. H264 frames come from x264 (`vendor/x264-src`,
built from source) through `native/h264.c`, with a keyframe whenever RTCP
asks for one, as Discord's client sends them, and otherwise at most a minute
apart: one every 3 seconds, the largest frame each time, froze viewers on a
busy upload; [Choosing the H264
encoder](#choosing-the-h264-encoder) says how it's set up and why. Discord Obsidian also
has VP8 (`native/vp8.c`, still built and tested); every Go Live sender known to
work sends H264. H264 goes as whole NAL units or FU-A fragments, H265 as whole
units or FU fragments (RFC 7798), and AV1 in AOM's [RTP payload
format](https://aomediacodec.github.io/av1-rtp-spec/v1.0.0.html), as
libwebrtc's packetizer sends it: OBU elements without their size fields,
temporal delimiters, tile lists and padding dropped, an element too large
for a packet carried on in the next, OBUs of different layers in packets of
their own, and a keyframe's first packet marked as starting a coded video
sequence (N). A frame with a malformed OBU isn't sent. DAVE leaves an AV1
frame's last OBU without its size, its trailer after it, and receivers keep
it so, since DAVE's decryptor reads the frame as it was sealed. Past a
frame's first 8 packets, while nothing waits to be sent, the rest are paced
at a quarter more than the target bitrate, as Discord's client sends: a
1080p keyframe of 200 sent at once can overflow buffers on the way.
Captured from Discord's client, a 20 KB keyframe of a 1.5 Mbit/s stream went
out over 86 ms, and no tenth of a second carried more than 2 Mbit/s. With
the encoders' eighth of a second buffer, the largest frame leaves in 100 ms.
Pacing at two and a half times the target and at least 40 Mbit/s sent
keyframes at four times a 10 Mbit/s upload, which dropped most of them. The
pacer keeps one schedule across frames, so a frame queued behind a keyframe
waits its turn, and keeps it in absolute time, so a timer that wakes late sends what
fell due meanwhile at once rather than slowing the stream; sleeping 1 ms
every 4 packets held 1080p144 below 30 fps on Windows, where each sleep took
15.6 ms. About a tenth of a second of frames may wait to be sent, at least
4 and at most 36. While more than half that waits, the worker skips
encoding frames, which leaves what viewers decode whole; a frame that
doesn't fit even so is dropped and the next is a keyframe. Keyframes come
at most every half second, however many viewers ask, as Discord's client was
captured answering picture loss indications: under loss, viewers asking for
one after each lost one kept the path full of the largest frames. Media connections ask for 4 MiB UDP buffers each way, and a send
the system has no room for drops the packet, as a congested path would:
Windows fails such a send (WSAENOBUFS) where other systems wait, and ending
the connection on it dropped streams and calls. Discord Obsidian keeps the last 2048 video packets, and when a viewer reports
some lost with a generic NACK (RTCP 205), resends up to 128 of them as
Discord's client does: on the RTX SSRC with RTX's payload type and numbering,
the original sequence number before the payload (RFC 4588), and the repaired
stream id (extension 12) in place of the stream id. Resent packets take at
most a quarter of the target bitrate each second; under heavy loss, resending
all that viewers asked for doubled what went out. An RTCP sender report goes
out every second.

Before sharing, the share button's menu picks a resolution and a frame rate:
one of the modes 480p, 720p, 1080p, 1440p or the screen's own size (up to 4K),
and any frame rate typed in, from 1 to 360 fps, or picked from 30, 60, 144,
240 and 360. On Windows a screen is captured through DXGI Desktop Duplication
(`crates/obsidian-core/src/duplication.rs`), which gives every frame the
screen presents, up to its refresh rate; WGC, which WebRTC's capturer uses,
gives about 60. So is a shared window while it fills its screen and is the
window in front, as a game being played is (`duplication::fronted_screen`,
checked every 250 ms): when it leaves the front, as on alt-tab, or stops
filling its screen, sharing moves to window capture before the screen shows
anything else, and back once the window returns. A game that wasn't in
front when sharing started gets duplication once it is. Window capture is
Windows Graphics Capture driven by Discord Obsidian (`window_capture.rs`),
as Discord's client captures a window: what the window draws, games
included, with other windows over it, at up to the window's rate on
Windows 11 24H2 (`MinUpdateInterval`), without the yellow border, and with
the pointer. Its Direct3D device is made on the adapter driving the
window's screen. WebRTC's window capturer made its own on the system's first
adapter, which on a machine with a virtual display driver, as game
streaming hosts install, isn't the GPU drawing the game, and games such as
Valorant shared black. A picture still black after 5 seconds is logged
("what's shared has been black for 5 seconds"). Frames go
out at the chosen rate at most, and only when the
screen changed, with the last one again every 100 ms while nothing does. The
pointer, which duplication leaves out, is drawn onto the screen's picture
while a frame is scaled and sent, and taken off again after; the worker keeps
one picture at the screen's size, not a copy for each frame. A game's
window that window capture can't open, minimized or filling its screen, as
one running as administrator may be, waits for the game to fill its screen
in front and shares that screen through duplication meanwhile, the last
picture staying while it's away (`wait_for_front`); WebRTC's capturer gave
games black or nothing. Cameras, screens duplication can't open, and other
windows Windows can't capture go through WebRTC's capturer. A frame within 4% of the stream's size each way, as a window
gaining or losing a row gives, is scaled to it instead of opening the
encoder again with a keyframe. While sharing, the worker asks Windows for
1 ms timers (`timeBeginPeriod`), which it otherwise rounds to 15.6 ms in
each process whatever a game sets, runs its capture thread above normal
priority, and puts duplication's GPU copies ahead of other work
(`SetGPUThreadPriority`), so a game keeping the processor and GPU busy
doesn't make the stream stutter.
The window's own preview of the stream updates at most 30 times a second.
NVENC encodes with preset p4 up to 240 fps and p3 above. Its "Higher quality" checkbox, which Discord's client
doesn't have, has x264 work harder on each frame. Discord Obsidian remembers the choice
beside the chosen audio devices (`devices.json`), and starts at 1080p30 without
it. Larger screens, and those of another shape, are scaled down to fit. Discord Obsidian
sends them at the bitrates Discord's client uses: at most 3.5 Mbit/s up to 720p
9 Mbit/s up to 1080p and 20 Mbit/s above, growing past 60 fps with the rate
to twice that at 240 fps and above (1080p144 at 13.2 Mbit/s), at most
40 Mbit/s, the cap op 12 announces, with the frame rate as its
`max_framerate`; a change of quality announces the new rate. The stream
starts at 4 Mbit/s, about the server's own first estimate, or the cap if
that's lower, and follows the packets lost on the way, as libwebrtc's
loss-based estimate does in Discord's client (`stream_connection::RateControl`):
each second it takes the larger of the loss the server's receiver reports
give for the video and the share of packets viewers' NACKs name, each once,
and grows 8% while under 2% is lost, reaching a 1080p144 cap in about 20 s,
holds up to 10% and while nothing reports, and above that drops by half the
fraction lost, never below 500 kbit/s. It grows only while the encoder uses
what it has: never past one and a half times what the server acknowledged
receiving, as libwebrtc limits an application-limited sender; a calm screen
sends far below its target. It remembers the bitrate heavy loss came at:
within 15% of it it grows 1% a second, and it stays under 90% of it until
90 seconds pass without loss, when that ceiling rises a tenth. Growing at
full speed ran into the same loss every 8 seconds, and creeping on at 1% a
second every 30 to 60, each time freezing viewers. Starting at a 1080p360 stream's
36 Mbit/s cap lost three packets in four on a home connection.

Discord's server also sends transport-wide congestion control feedback
(RTCP 205, format 15), five to ten times a second, as it does to Discord's
client, which a capture of it shows following: when each packet with a
transport-wide sequence number arrived, by the server's clock. From it
Discord Obsidian runs the delay-based estimate Discord's client runs,
libwebrtc's GCC (`congestion.rs`): packets sent within 5 ms make a group;
how much further apart consecutive groups arrived than they were sent is the
queueing delay they gained; a trendline over the last 20 groups' delay,
against a threshold that adapts to the path, says when a queue builds. Then
the rate drops to 85% of what the server acknowledged receiving, holds while
the queue drains, and otherwise grows 8% a second, or 2% within a tenth of
the capacity the last overuse showed, and never past one and a half times
what's acknowledged, beyond that capacity. So the stream backs off as a queue
starts to build, before packets are lost, where the loss-based estimate only
learns of a full path from loss. The stream aims for the lower of the two,
as libwebrtc does, and for the loss-based one alone until feedback comes,
which is logged ("following the server's transport-wide feedback"). The
encoder follows when the target and its bitrate differ by more than a tenth.
While the target is low, fewer frames go, each with enough bits, as
libwebrtc keeps a screen's resolution and lowers its frame rate
(`steady_fps`): every frame while each gets at least half what a 60 fps
frame gets at the cap, and down to 30 a second below that, 1080p144 sending
66 at 5 Mbit/s. The encoders size each frame for the rate they opened at,
so they're asked for as much more as frames are left out.

The server's own estimate (REMB) is logged but not followed, which is where
Discord Obsidian chooses to differ from Discord's client: it only grows past
what it receives once the sender probes with padding, and without probing it
held 1080p144 streams at 1.5 Mbit/s for as long as they ran. Discord's client,
captured streaming a moving 1440p screen to the same server, also sent a
flat 1.5 Mbit/s. Discord's client allows accounts without Nitro
720p30, but trials 1080p30 at 3.5 or 5 Mbit/s for them. The worker converts
captured frames to I420 with libyuv, scales them with libyuv's filtering
scaler, and encodes those; the preview is the same picture scaled down to 720p.

Discord shows a still of each stream, in the call and to people outside it.
Like Discord's client, Discord Obsidian makes one once the stream connects and the
picture isn't black any more (a screen is black until what's shared draws),
then every 5 minutes: the picture scaled down to fit 512 by 288, a JPEG at
quality 92. The worker reports it as a data URI, and Discord Obsidian uploads it with
`POST /streams/{stream key}/preview`. A failed upload is tried again a minute
later, up to three times; a newer still replaces an upload not done yet, and
stopping the share cancels it.

A shared screen carries the sound every other application plays, as Discord's
client and Vesktop share it; Discord Obsidian's own sound stays out, so viewers never hear
the call echoed back (`stream_audio`, with `audio_io::others` hearing the
applications). On Linux, PulseAudio or PipeWire's
PulseAudio server monitors each application's playback stream on its own
(`pa_stream_set_monitor_stream`, as pavucontrol's level meters do), and
streams from Discord Obsidian's processes are skipped by process id. So are other Discord
clients' (Discord, Vesktop, Equibop, WebCord, Legcord, ArmCord, by application
name or program): with one of them in the call too, its sound echoed the call
back to the people in it. The worker mixes them every 20 ms, waiting until an
application has 40 ms queued and keeping at most 200 ms of each, and encodes
Opus at 64 kbit/s, what Discord's client gives a stream's sound. Frames go out
as plain Opus RTP on the stream's audio SSRC, DAVE-encrypted, with sender
reports, and op 5 marks them as shared sound (speaking flag 2). Nothing is
sent while no application plays. On Windows, WASAPI's process loopback hears
everything but Discord Obsidian's own process tree as one stream, from Windows 10's 2004
update on; it can't leave other Discord clients out. Other systems share
video only for now, and so does the test pattern. Two ignored tests check the capture against the
machine's sound server with a quiet tone: another process's is heard, and
Discord Obsidian's own is not.

Each video packet carries the one-byte extensions a Discord client sends: a
transport-wide sequence number (id 5), playout delay (6), screen content
type (7) and the stream id `100` (11). The stream id is what ties a packet to
the stream op 12 declared. These ids aren't the ones in Discord's WebRTC
SDP, where 5 is playout delay: with only playout delay as id 5, Discord's
server drops every packet, and viewers time out with error 2012.

DAVE follows songbird's handshake (`dave.rs`) and encrypts each frame once the
group is ready. A stream's MLS group isn't the voice channel's: its id is the
RTC server id minus one, which Discord-video-stream found too. If the server's
proposals or commits name another group, Discord Obsidian moves to it. The vendored
davey's `set_channel_id` keeps the key package already sent, so the streamer's
welcome still opens. Discord Obsidian accepts the people op 11 names into the group, and
also those named in video attributes (op 12) or in the stream key.

With `OBSIDIAN_LOG=debug`, Discord Obsidian logs every 5 s while sharing what it sent and the
RTCP that came back (`stream sent`): receiver reports with the highest
sequence number Discord's server got and its echo of our sender reports
(`lsr`), its bitrate estimate (REMB), keyframe requests, and the bitrate
aimed for. Frames the worker skipped or dropped while the network was
behind are logged at info, summed every 10 s when there were any (`stream
frames held back`). Each change of a tenth or more in the bitrate is logged at info
(`stream bitrate`), with each estimate (`delay_kbps`, `loss_kbps`), what the
delay last said and the bitrate the server acknowledged, the loss, and each
measure of it: what the server reports reached it from here (`upload_loss_percent`), and the share of packets viewers asked for again (`viewer_nack_percent`), which counts what was lost on the way to them too. The voice connection logs the voice server's RTCP too, to compare
with a sender Discord accepts. Messages the stream server sends are logged
too, except those carrying keys.

Watching works the same way in reverse. Clicking someone's LIVE badge, or their
tile in the call, sends op 20 and starts a worker with a watch job. The worker
connects as a viewer, offers AV1, H265, H264 and VP8 for decoding only (H265
only where it's built, on Linux and Windows), and asks for video with op 15. It
learns the streamer's SSRCs and codec from op 12 and op 14, then puts RTP
packets back in order (`rtp::Reorder`) and together into frames
(`rtp::Assembler`: VP8 descriptors, H264 single units, STAP-A and FU-A, and
H265 single units, aggregation and fragmentation units, as Annex B; AV1's
OBU elements as OBUs, the last without its size, as DAVE's decryptor needs
it and dav1d takes it). As
Discord's client does, it asks for a missing packet again at once with a
generic NACK, again every 80 ms up to three times, and takes the resend from
the streamer's RTX SSRC (RFC 4588); padding there, which probes the path, is
dropped. Packets after a gap wait for it at most 300 ms. A frame still missing
a packet is dropped, and so is everything after it until the next keyframe. So
are frames while none has come yet, and after one the decoder was too far
behind to take. The connection asks for a keyframe with a picture loss
indication, at most every 500 ms. It sends a receiver report every second
(packets lost, the highest received, and the streamer's last sender report
echoed). With `OBSIDIAN_LOG=debug` it logs what it received every 5 s (`stream
received`): packets out of order, resent, recovered, given up on, and keyframes
asked for. The worker decodes with libvpx or with the FFmpeg H264 decoder in
libwebrtc's archive (`native/decode.c`), AV1 with the dav1d in the same
archive (`native/av1.c`), a picture out for each frame in, or H265 with the
HEVC decoder of `vendor/ffmpeg-src`'s FFmpeg (`native/hevc.c`), which
libwebrtc's lacks, and writes RGBA at up to 1080p into the shared frames.
dav1d takes 0.07 of a core for 1080p30 AV1 at 3.6 Mbit/s, against 0.1 for
H265 at the same bitrate. That FFmpeg is built with
its x86-64 assembly, without which H265 took over three times the processor
time. Watching is possible only in the call you're in, so clicking someone
sharing in another channel joins it first, once asked, and watches once the
call is connected; Discord drops a watch sent before. The stream's sound, Opus
on the streamer's audio SSRC, is DAVE-decrypted as audio and played by the
worker through `audio_io.rs` on the call's speakers (`stream_audio::Player`),
about 100 ms queued; being deafened silences it, which Discord Obsidian tells the worker
with a `hear` control. Discord Obsidian's workers play as "Discord Obsidian", which sharing
leaves out, so a stream you watch never reaches one you share. Watching a
stream from Discord's own client works: H264 with DAVE. Discord's client shows
Discord Obsidian's stream at 1080p with its sound, without the call's.

The fake server (`testing/fake_discord.ts`) checks what a stream sends, without
DAVE, and streams its recordings to viewers; its header lists the controls
that add a viewer or change a stream's codec.

## Choosing the H264 encoder

OpenH264, in libwebrtc's archive, doesn't suit Go Live: on anything that moves
it drops frames rather than lower their quality, and with frame skipping off
its rate control runs far past the target.

x264 encodes every frame within the bitrate. `native/h264.c` uses its superfast
preset, or faster with "Higher quality", tuned for zero latency (a frame out
for each frame in, no B-frames or lookahead, a slice for each of up to four
threads), in Main profile, which Discord's client sends (profile-level-id
`4d0033`). Superfast is the default because it costs little processor time;
faster takes about twice as much for about NVENC's quality. Adaptive
quantization is off: it was the one setting that helped both screens and games
against x264's defaults. The bitrate peaks at the target over an eighth of a
second's buffer, about 4 frames at 30 fps, so no frame bursts past what the
path carries; a second's buffer let a second reach twice the cap, and a
two-frame buffer starved keyframes. Capped CRF ran past Discord's cap.

## Encoding on the GPU

With "Encode streams with the GPU" in Settings, Advanced, on unless turned off,
a stream's H264 goes to the graphics card's encoder where it has one:
`native/hwenc.c` opens FFmpeg's NVENC encoder, then on Windows its AMF one
(AMD) and on Linux its VA-API one (AMD and Intel), then on Windows the GPU's
encoder Media Foundation lists, which every GPU maker's driver provides:
Intel's Quick Sync, on Arc cards and the graphics in Intel's processors, and
any GPU the others don't open on. Only a GPU's: Microsoft's own encoder runs
on the processor, where x264 does better, and x264 encodes when none opens.
It's on by default because it takes almost no processor time and looks better
than x264 at low bitrates. Media Foundation's encoders get the same settings
where the driver has them (low latency, the display remoting scenario,
variable bitrate under a peak, the bitrate changed as the stream runs, Main
profile, forced keyframes), and the access unit delimiter they start each
frame with is left out, as every other encoder here and Discord's client
send none. `OBSIDIAN_HWENC=_mf` tries only the encoders with that in their
name, to try Media Foundation's on a GPU whose own would open first; on an
RTX 5070 Ti it encoded H264, H265 and AV1 through NVIDIA's.
The setting lives in `layout.json` and reaches the worker with each
`Job::Share`; the camera stays on x264. When the GPU's H264 encoder fails
during a stream, x264 takes over from that frame with a keyframe. The worker
logs which encoder each stream got ("encoding the stream", `engine` and
`codec`), and for each codec none opened for, why each GPU encoder failed
("no GPU encoder opened"). The call's Stream tab shows the encoder and the
codec Discord chose ([Voice](voice.md)).

FFmpeg is built from `vendor/ffmpeg-src` into the one binary, with only its
hardware encoders and every symbol prefixed so it can't meet the FFmpeg
inside libwebrtc. NVIDIA's driver, AMD's AMF runtime and libva load only
when an encoder opens: the binary starts without any of them, and needs
NVIDIA's driver 520 or newer for NVENC, and for AMF an AMD driver with AMF
1.5.2 or newer, from about May 2026 on; an older one fails to open, and x264
encodes.

VA-API tries each render node (`/dev/dri/renderD128` on) with the libva
driver its kernel driver calls for: radeonsi for amdgpu, iHD then i965 for
Intel's. Naming it overrides `LIBVA_DRIVER_NAME`, which NixOS and others set
to `nvidia` for browsers, and which would otherwise load NVIDIA's decoding-only
driver for every GPU, AMD's and Intel's too. NVIDIA's nodes are skipped:
NVENC encodes there. NixOS keeps libva itself off the library path, so the
Nix package's wrapper and the dev shell put it there (`runtimeLibraries` in
`nix/native.nix`); it's no build input, or webrtc-sys would build LiveKit's
own VA-API encoder into libwebrtc. The AppImage takes the system's libva,
and the Arch package lists it as optional.

The settings match x264's where they can: low-latency tuning (NVENC's p4
preset, `ll`; AMF's `lowlatency` usage and balanced preset), no B-frames, a
frame out for each frame in, Main profile, keyframes when asked and at most
a minute apart otherwise, and variable bitrate capped at the target over an eighth of a second, which
leaves a calm screen below the cap rather than padding it. NVENC, AMF and
VA-API change their bitrate as they run, without the IDR frame FFmpeg would
insert, with a patch to FFmpeg for each (AMF's sets the target, peak and buffer
on the running encoder). "Higher quality" changes nothing on the GPU: NVENC's
slowest preset looked no better than p4.

The GPU's encoder is most of the worker's memory, so it's opened to take as
little as it can. On Windows NVENC's session opens on a Direct3D 11 device
rather than a CUDA context (a patch to FFmpeg, with
`vendor/ffmpeg-src/src/nvenc_d3d11.c`), which encodes the same and takes
over 100 MiB less; CUDA opens it only when no device does. NVENC keeps one
input and output buffer, as one frame is in it at a time, where FFmpeg's
default is four, and reads the worker's I420 planes as they are, without a
frame of FFmpeg's in between. Of the codecs offered, only the first is opened
at the stream's size when the stream starts; the others are opened at 640
by 360 to see that they open, since a closed encoder doesn't give all its
memory back.

Where the GPU's encoder encodes AV1 or H265, the stream offers them too,
first, AV1 before H265, as Discord's client orders them: each looks better
than the one after it at the same bitrate. When the stream starts, the
worker opens each to see that it opens, and keeps the first: an encoder
FFmpeg has isn't enough, the GPU and its driver must encode the codec. There
is no AV1 encoder on the processor, so with "Encode streams with the GPU"
off, the stream offers H264 alone. Discord picks the codec Discord Obsidian offers
first that every viewer's client decodes (op 4's `video_codec`), and when
someone who can't decode it starts watching, switches the stream with op
14; Discord Obsidian then opens the other encoder and starts it with a keyframe,
dropping frames still in the old codec. When AV1 or H265 fails mid-stream,
or doesn't open again for a new size or after a switch, Discord Obsidian withdraws
it with its own op 14 `codecs`, marking it not to encode, and goes on in the
next codec offered, H264 last. Another Discord Obsidian watching keeps the stream in
AV1 or H265. Which of Discord's own clients decode H265 and AV1 isn't
checked yet.

AV1 encodes on NVIDIA's RTX 40 series and newer (`av1_nvenc`), on AMD's
RX 7000 series and newer and the Radeon 700M graphics, through AMF on
Windows (`av1_amf`) and through Mesa's radeonsi on Linux (`av1_vaapi`), and
on Intel's Arc and Core Ultra graphics through VA-API (iHD) on Linux and Media Foundation on Windows (`av1_mf`). GPUs
before those have no AV1 encoder, so the stream offers H265 and H264. On an
RX 7900 XTX with AMD's driver 32.0.31041.1004 on Windows, `native/hwenc.c`,
built on its own with FFmpeg's Windows build, encoded with `av1_amf`,
`h264_amf` and `hevc_amf`, forced keyframes and a bitrate change included,
and dav1d decoded the AV1; a whole Discord Obsidian hasn't run there yet, nor AV1 on
Linux, NVIDIA's or Intel's. The packetizer was checked against libaom's AV1
of the test recording, which dav1d decodes to the same pictures after the
packets are put back together, and against DAVE, which decrypts it.

AMD's RX 7000 series codes AV1 in blocks of 64 by 16 pixels, and the
picture's size rounds up: 320x180 came out as 320x192, and AMD gives
1920x1080 as 1920x1082. FFmpeg hands the crop on beside the stream, which
RTP can't carry, so viewers may see the extra rows at the bottom; sizes
such as 1280x720 fit the blocks. See AMD's [alignment
documentation](https://github.com/GPUOpen-LibrariesAndSDKs/AMF/wiki/AV1-Encoder#av1-specific-api)
and [the issue about it](https://github.com/GPUOpen-LibrariesAndSDKs/AMF/issues/423).
The Stream tab gives the size Discord Obsidian encodes, before the rounding.

Wine has no AMF runtime, so there AMF fails to open and x264 encodes.
