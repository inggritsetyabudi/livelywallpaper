#import <Cocoa/Cocoa.h>
#import <AVFoundation/AVFoundation.h>
#import <WebKit/WebKit.h>
#import <Metal/Metal.h>
#import <MetalKit/MetalKit.h>
#import <IOKit/ps/IOPowerSources.h>
#import <IOKit/ps/IOPSKeys.h>
#include "lively_native.h"

#pragma mark - Data Structures

@interface LivelyDesktopWindow : NSWindow
@end

@implementation LivelyDesktopWindow
- (BOOL)canBecomeKeyWindow { return NO; }
- (BOOL)canBecomeMainWindow { return NO; }
@end

typedef struct {
    AVQueuePlayer *player;
    AVPlayerLooper *looper;
    AVPlayerLayer *playerLayer;
    NSView *containerView;
    float currentVolume;
    float currentRate;
} LivelyVideoContext;

typedef struct {
    WKWebView *webView;
    NSView *containerView;
} LivelyWebContext;

@interface LivelyShaderRenderer : NSObject <MTKViewDelegate>
@property (nonatomic, strong) id<MTLDevice> device;
@property (nonatomic, strong) id<MTLCommandQueue> commandQueue;
@property (nonatomic, strong) id<MTLRenderPipelineState> pipelineState;
@property (nonatomic, strong) id<MTLBuffer> uniformBuffer;
@property (nonatomic, assign) CFTimeInterval startTime;
@property (nonatomic, assign) BOOL paused;
@property (nonatomic, assign) CGPoint mousePos;
@property (nonatomic, strong) NSData *audioData;
@end

typedef struct {
    float resolution[2];
    float time;
    float timeDelta;
    int32_t frame;
    float mouse[4]; // xy = current, zw = click
    float audioBands[128];
} ShaderUniforms;

@implementation LivelyShaderRenderer {
    uint32_t _frameCount;
    CFTimeInterval _lastFrameTime;
    ShaderUniforms _uniforms;
}

- (instancetype)initWithDevice:(id<MTLDevice>)device {
    self = [super init];
    if (self) {
        _device = device;
        _commandQueue = [device newCommandQueue];
        _startTime = CACurrentMediaTime();
        _lastFrameTime = _startTime;
        _frameCount = 0;
        _paused = NO;
        _mousePos = CGPointZero;
        _uniformBuffer = [device newBufferWithLength:sizeof(ShaderUniforms)
                                             options:MTLResourceStorageModeShared];
    }
    return self;
}

- (void)mtkView:(MTKView *)view drawableSizeWillChange:(CGSize)size {
}

- (void)drawInMTKView:(MTKView *)view {
    if (_paused || !_pipelineState) return;

    CFTimeInterval now = CACurrentMediaTime();
    float time = (float)(now - _startTime);
    float dt = (float)(now - _lastFrameTime);
    _lastFrameTime = now;
    _frameCount++;

    _uniforms.resolution[0] = (float)view.drawableSize.width;
    _uniforms.resolution[1] = (float)view.drawableSize.height;
    _uniforms.time = time;
    _uniforms.timeDelta = dt;
    _uniforms.frame = _frameCount;
    _uniforms.mouse[0] = (float)_mousePos.x;
    _uniforms.mouse[1] = (float)_mousePos.y;

    if (_audioData && _audioData.length >= sizeof(float) * 128) {
        memcpy(_uniforms.audioBands, [_audioData bytes], sizeof(float) * 128);
    }

    memcpy([_uniformBuffer contents], &_uniforms, sizeof(ShaderUniforms));

    MTLRenderPassDescriptor *passDesc = view.currentRenderPassDescriptor;
    if (!passDesc) return;

    id<MTLCommandBuffer> cmdBuf = [_commandQueue commandBuffer];
    id<MTLRenderCommandEncoder> encoder = [cmdBuf renderCommandEncoderWithDescriptor:passDesc];

    [encoder setRenderPipelineState:_pipelineState];
    [encoder setVertexBuffer:_uniformBuffer offset:0 atIndex:0];
    [encoder setFragmentBuffer:_uniformBuffer offset:0 atIndex:0];
    [encoder drawPrimitives:MTLPrimitiveTypeTriangleStrip vertexStart:0 vertexCount:4];
    [encoder endEncoding];

    [cmdBuf presentDrawable:view.currentDrawable];
    [cmdBuf commit];
}
@end

typedef struct {
    MTKView *mtkView;
    LivelyShaderRenderer *renderer;
} LivelyShaderContext;

typedef struct {
    NSImageView *imageView;
    NSImage *image;
} LivelyImageContext;

#pragma mark - App Lifecycle

static TrayCallback g_tray_cb = NULL;
static NSStatusItem *g_status_item = nil;

bool lively_init_app(void) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
        return true;
    }
}

void lively_run_loop_run(void) {
    @autoreleasepool {
        [NSApp run];
    }
}

void lively_run_loop_stop(void) {
    @autoreleasepool {
        [NSApp stop:nil];
    }
}

bool lively_is_main_thread(void) {
    return [NSThread isMainThread];
}

void lively_pump_events(double timeout_secs) {
    @autoreleasepool {
        NSDate *limitDate = [NSDate dateWithTimeIntervalSinceNow:timeout_secs];
        NSEvent *event = [NSApp nextEventMatchingMask:NSEventMaskAny
                                            untilDate:limitDate
                                               inMode:NSDefaultRunLoopMode
                                              dequeue:YES];
        if (event) {
            [NSApp sendEvent:event];
            [NSApp updateWindows];
        }
    }
}

#pragma mark - Screens

uint32_t lively_get_screen_count(void) {
    @autoreleasepool {
        return (uint32_t)[[NSScreen screens] count];
    }
}

bool lively_get_screen_bounds(uint32_t index, double *x, double *y, double *w, double *h) {
    @autoreleasepool {
        NSArray<NSScreen *> *screens = [NSScreen screens];
        if (index >= [screens count]) return false;
        NSRect r = [screens[index] frame];
        if (x) *x = r.origin.x;
        if (y) *y = r.origin.y;
        if (w) *w = r.size.width;
        if (h) *h = r.size.height;
        return true;
    }
}

// Returns a malloc'd copy the caller must free with lively_free_string().
// A `const char*` into an autoreleased NSString would be cheaper, but that
// memory dies with the @autoreleasepool — the copy is what makes the pointer
// safe to hand across the FFI boundary.
const char* lively_get_screen_name(uint32_t index) {
    @autoreleasepool {
        NSArray<NSScreen *> *screens = [NSScreen screens];
        if (index >= [screens count]) return strdup("Display");
        NSString *name = [screens[index] localizedName];
        return strdup([name UTF8String]);
    }
}

void lively_free_string(char* s) {
    if (s) free(s);
}

#pragma mark - Desktop Window

void* lively_create_desktop_window(uint32_t screen_index) {
    @autoreleasepool {
        NSArray<NSScreen *> *screens = [NSScreen screens];
        if (screen_index >= [screens count]) return NULL;

        NSScreen *screen = screens[screen_index];
        NSRect frame = [screen frame];

        LivelyDesktopWindow *win = [[LivelyDesktopWindow alloc] initWithContentRect:frame
                                                                         styleMask:NSWindowStyleMaskBorderless
                                                                           backing:NSBackingStoreBuffered
                                                                             defer:NO
                                                                            screen:screen];
        // Desktop window level: above system desktop picture, behind desktop icons
        // kCGDesktopWindowLevel is -2147483623; +1 places it right over desktop wallpaper
        [win setLevel:(kCGDesktopWindowLevel + 1)];
        [win setCollectionBehavior:NSWindowCollectionBehaviorCanJoinAllSpaces |
                                   NSWindowCollectionBehaviorStationary |
                                   NSWindowCollectionBehaviorIgnoresCycle];
        [win setHidesOnDeactivate:NO];
        [win setCanBecomeVisibleWithoutLogin:YES];
        [win setIgnoresMouseEvents:YES];
        [win setOpaque:YES];
        [win setBackgroundColor:[NSColor blackColor]];

        NSView *contentView = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, frame.size.width, frame.size.height)];
        [contentView setWantsLayer:YES];
        [win setContentView:contentView];

        [win orderFront:nil];
        return (__bridge_retained void*)win;
    }
}

void lively_destroy_desktop_window(void* win_ptr) {
    if (!win_ptr) return;
    @autoreleasepool {
        NSWindow *win = (__bridge_transfer NSWindow*)win_ptr;
        [win orderOut:nil];
        [win close];
    }
}

void lively_set_window_level(void* win_ptr, int32_t level_offset) {
    if (!win_ptr) return;
    @autoreleasepool {
        NSWindow *win = (__bridge NSWindow*)win_ptr;
        [win setLevel:(kCGDesktopWindowLevel + level_offset)];
    }
}

void lively_set_window_interactive(void* win_ptr, bool interactive) {
    if (!win_ptr) return;
    @autoreleasepool {
        NSWindow *win = (__bridge NSWindow*)win_ptr;
        [win setIgnoresMouseEvents:!interactive];
    }
}

#pragma mark - Video Engine (AVFoundation)

void* lively_create_video_view(void* win_ptr, const char* file_path, LivelyScaleMode scale_mode) {
    if (!win_ptr || !file_path) return NULL;
    @autoreleasepool {
        NSWindow *win = (__bridge NSWindow*)win_ptr;
        NSString *pathStr = [NSString stringWithUTF8String:file_path];
        NSURL *url = [NSURL fileURLWithPath:pathStr];

        AVPlayerItem *item = [AVPlayerItem playerItemWithURL:url];
        AVQueuePlayer *player = [AVQueuePlayer queuePlayerWithItems:@[item]];
        AVPlayerLooper *looper = [AVPlayerLooper playerLooperWithPlayer:player templateItem:item];

        AVPlayerLayer *playerLayer = [AVPlayerLayer playerLayerWithPlayer:player];
        switch (scale_mode) {
            case SCALE_ASPECT_FILL:
                playerLayer.videoGravity = AVLayerVideoGravityResizeAspectFill;
                break;
            case SCALE_ASPECT_FIT:
                playerLayer.videoGravity = AVLayerVideoGravityResizeAspect;
                break;
            case SCALE_STRETCH:
            default:
                playerLayer.videoGravity = AVLayerVideoGravityResize;
                break;
        }

        NSView *root = [win contentView];
        playerLayer.frame = root.bounds;
        playerLayer.autoresizingMask = kCALayerWidthSizable | kCALayerHeightSizable;

        NSView *videoView = [[NSView alloc] initWithFrame:root.bounds];
        [videoView setWantsLayer:YES];
        [videoView.layer addSublayer:playerLayer];
        videoView.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;

        [root addSubview:videoView];
        [player play];

        LivelyVideoContext *ctx = (LivelyVideoContext*)malloc(sizeof(LivelyVideoContext));
        ctx->player = player;
        ctx->looper = looper;
        ctx->playerLayer = playerLayer;
        ctx->containerView = videoView;
        ctx->currentVolume = 1.0f;
        ctx->currentRate = 1.0f;

        CFRetain((__bridge CFTypeRef)player);
        CFRetain((__bridge CFTypeRef)looper);
        CFRetain((__bridge CFTypeRef)playerLayer);
        CFRetain((__bridge CFTypeRef)videoView);

        return ctx;
    }
}

// Playback state, for diagnostics and for the UI to show "loading" / "failed"
// instead of silently displaying nothing.
typedef enum {
    LIVELY_PLAYBACK_UNKNOWN = 0,
    LIVELY_PLAYBACK_LOADING = 1,
    LIVELY_PLAYBACK_READY = 2,
    LIVELY_PLAYBACK_PLAYING = 3,
    LIVELY_PLAYBACK_PAUSED = 4,
    LIVELY_PLAYBACK_FAILED = 5,
    LIVELY_PLAYBACK_ENDED = 6
} LivelyPlaybackState;

int32_t lively_video_get_state(void* player_ptr) {
    if (!player_ptr) return LIVELY_PLAYBACK_UNKNOWN;
    LivelyVideoContext *ctx = (LivelyVideoContext*)player_ptr;
    AVPlayerItem *item = ctx->player.currentItem;
    if (!item) return LIVELY_PLAYBACK_UNKNOWN;

    if (item.status == AVPlayerItemStatusFailed) return LIVELY_PLAYBACK_FAILED;
    if (item.status == AVPlayerItemStatusUnknown) return LIVELY_PLAYBACK_LOADING;

    // `timeControlStatus` is the accurate answer for "is it moving right now";
    // comparing `rate` alone misses the paused-at-end case.
    switch (ctx->player.timeControlStatus) {
        case AVPlayerTimeControlStatusPlaying:
            return LIVELY_PLAYBACK_PLAYING;
        case AVPlayerTimeControlStatusWaitingToPlayAtSpecifiedRate:
            return LIVELY_PLAYBACK_LOADING;
        case AVPlayerTimeControlStatusPaused:
            break;
    }

    CMTime end = item.duration;
    if (CMTIME_IS_NUMERIC(end) && CMTimeGetSeconds(end) > 0) {
        CMTime now = ctx->player.currentTime;
        if (CMTIME_IS_NUMERIC(now) && CMTimeGetSeconds(now) >= CMTimeGetSeconds(end) - 0.05) {
            return LIVELY_PLAYBACK_ENDED;
        }
    }
    return LIVELY_PLAYBACK_PAUSED;
}

// Current position in seconds; negative when unknown.
double lively_video_get_position(void* player_ptr) {
    if (!player_ptr) return -1.0;
    LivelyVideoContext *ctx = (LivelyVideoContext*)player_ptr;
    CMTime now = ctx->player.currentTime;
    if (!CMTIME_IS_NUMERIC(now)) return -1.0;
    return CMTimeGetSeconds(now);
}

// Duration in seconds; negative when unknown (streams, still loading).
double lively_video_get_duration(void* player_ptr) {
    if (!player_ptr) return -1.0;
    LivelyVideoContext *ctx = (LivelyVideoContext*)player_ptr;
    CMTime duration = ctx->player.currentItem.duration;
    if (!CMTIME_IS_NUMERIC(duration)) return -1.0;
    return CMTimeGetSeconds(duration);
}

// Point an existing video view at a different file.
//
// Rebuilding the player, looper and layer for every wallpaper change leaks
// roughly 1.3 MB per switch and never plateaus. Swapping just the item keeps
// the expensive objects alive for the life of the screen and makes switching
// instant.
int lively_video_set_source(void* player_ptr, const char* file_path) {
    if (!player_ptr || !file_path) return 0;
    @autoreleasepool {
        LivelyVideoContext *ctx = (LivelyVideoContext*)player_ptr;

        NSURL *url = [NSURL fileURLWithPath:[NSString stringWithUTF8String:file_path]];
        AVPlayerItem *item = [AVPlayerItem playerItemWithURL:url];

        // The looper retains the template item it was built with, so it must go
        // before the queue does — and two loopers on one player is undefined.
        if (ctx->looper) {
            CFRelease((__bridge CFTypeRef)ctx->looper);
            ctx->looper = nil;
        }
        [ctx->player removeAllItems];
        [ctx->player replaceCurrentItemWithPlayerItem:nil];

        // `insertItem:afterItem:` throws NSInvalidArgumentException if the item
        // is already queued. An Objective-C exception unwinding into Rust aborts
        // the process outright ("Rust cannot catch foreign exceptions"), so the
        // precondition is checked rather than discovered.
        if (![ctx->player canInsertItem:item afterItem:nil]) {
            NSLog(@"[Lively] video item rejected by the queue; recreating instead");
            return 0;
        }
        [ctx->player insertItem:item afterItem:nil];

        AVPlayerLooper *looper = [AVPlayerLooper playerLooperWithPlayer:ctx->player templateItem:item];
        if (!looper) return 0;

        // Explicit +1 to match what the destroy path gives back.
        CFRetain((__bridge CFTypeRef)looper);
        ctx->looper = looper;

        [ctx->player seekToTime:kCMTimeZero];
        [ctx->player play];
        return 1;
    }
}

void lively_video_set_volume(void* player_ptr, float volume) {
    if (!player_ptr) return;
    LivelyVideoContext *ctx = (LivelyVideoContext*)player_ptr;
    ctx->currentVolume = volume;
    [ctx->player setVolume:volume];
}

void lively_video_set_playback_rate(void* player_ptr, float rate) {
    if (!player_ptr) return;
    LivelyVideoContext *ctx = (LivelyVideoContext*)player_ptr;
    ctx->currentRate = rate;
    [ctx->player setRate:rate];
}

void lively_video_seek(void* player_ptr, double percent) {
    if (!player_ptr) return;
    LivelyVideoContext *ctx = (LivelyVideoContext*)player_ptr;
    CMTime duration = ctx->player.currentItem.duration;
    if (CMTIME_IS_VALID(duration)) {
        double seconds = CMTimeGetSeconds(duration) * (percent / 100.0);
        CMTime target = CMTimeMakeWithSeconds(seconds, NSEC_PER_SEC);
        [ctx->player seekToTime:target];
    }
}

void lively_video_play(void* player_ptr) {
    if (!player_ptr) return;
    LivelyVideoContext *ctx = (LivelyVideoContext*)player_ptr;
    [ctx->player play];
}

void lively_video_pause(void* player_ptr) {
    if (!player_ptr) return;
    LivelyVideoContext *ctx = (LivelyVideoContext*)player_ptr;
    [ctx->player pause];
}

// Teardown order matters here.
//
// The view holds the layer as a sublayer and the layer holds the player, so
// releasing our own retains first would only decrement counters that something
// else still owns — the objects would survive, still reachable, and the memory
// would climb on every wallpaper change. Detaching the hierarchy *first* drops
// every owner to zero, then our explicit retains are the last reference.
void lively_destroy_video_view(void* player_ptr) {
    if (!player_ptr) return;
    @autoreleasepool {
        LivelyVideoContext *ctx = (LivelyVideoContext*)player_ptr;

        [ctx->player pause];

        // The queue is not the only path to the item: `currentItem` keeps one
        // alive too, and the looper holds the template. Clearing all three is
        // what actually lets AVFoundation release the asset and its decoded
        // frames — skipping any of them leaves roughly a megabyte behind per
        // wallpaper switch.
        [ctx->player removeAllItems];
        [ctx->player replaceCurrentItemWithPlayerItem:nil];

        // Detach each link so our explicit retain becomes the last one.
        // Do NOT nil out `containerView.layer`: that would drop the whole
        // sublayer chain and turn the CFRelease calls below into over-releases.
        [ctx->playerLayer removeFromSuperlayer];
        [ctx->containerView removeFromSuperview];

        // CFRelease(NULL) traps, so each handle is checked: a context can be
        // torn down after a partially failed setup.
        if (ctx->player)        CFRelease((__bridge CFTypeRef)ctx->player);
        if (ctx->looper)        CFRelease((__bridge CFTypeRef)ctx->looper);
        if (ctx->playerLayer)   CFRelease((__bridge CFTypeRef)ctx->playerLayer);
        if (ctx->containerView) CFRelease((__bridge CFTypeRef)ctx->containerView);
        free(ctx);
    }
}


/*
 * Runs a block, converting any Objective-C exception into `fallback`.
 *
 * A Rust `catch_unwind` cannot intercept a foreign exception: unwinding one
 * into a Rust frame aborts the process outright. Everything crossing the FFI
 * boundary that can plausibly throw -- WebKit in particular -- is therefore
 * guarded here, so a bad wallpaper fails one command instead of killing the
 * daemon.
 */
static void *lively_guarded(void *(^block)(void), void *fallback) {
    @try {
        return block();
    } @catch (NSException *e) {
        NSLog(@"[Lively] caught %@: %@", e.name, e.reason);
        return fallback;
    }
}

// Forward declarations: the guarded entry points sit above their bodies.
static void *create_web_view_impl(void *win_ptr, const char *url_or_path, bool is_local_file);
static void destroy_web_view_impl(void *web_ptr);

#pragma mark - Web Engine (WKWebView)

void* lively_create_web_view(void* win_ptr, const char* url_or_path, bool is_local_file) {
    if (!win_ptr || !url_or_path) return NULL;
    return lively_guarded(^{ return create_web_view_impl(win_ptr, url_or_path, is_local_file); }, NULL);
}

static void *create_web_view_impl(void* win_ptr, const char* url_or_path, bool is_local_file) {
    @autoreleasepool {
        NSWindow *win = (__bridge NSWindow*)win_ptr;
        WKWebViewConfiguration *config = [[WKWebViewConfiguration alloc] init];

        // Enable WebGL, autoplay, and dev tools
        [config.preferences setValue:@YES forKey:@"allowFileAccessFromFileURLs"];
        config.mediaTypesRequiringUserActionForPlayback = WKAudiovisualMediaTypeNone;

        // Lively JS API Injection
        NSString *livelyBridgeScript =
            @"window.livelyPropertyListener = window.livelyPropertyListener || function(name, val) {};\n"
            @"window.livelyAudioListener = window.livelyAudioListener || function(audioArray) {};\n"
            @"window.livelyCurrentTrack = window.livelyCurrentTrack || function(track) {};\n"
            @"window.livelyWallpaperPlaybackChanged = window.livelyWallpaperPlaybackChanged || function(state) {};\n";

        WKUserScript *script = [[WKUserScript alloc] initWithSource:livelyBridgeScript
                                                      injectionTime:WKUserScriptInjectionTimeAtDocumentStart
                                                   forMainFrameOnly:NO];
        [config.userContentController addUserScript:script];

        NSView *root = [win contentView];
        WKWebView *webView = [[WKWebView alloc] initWithFrame:root.bounds configuration:config];
        webView.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        [webView setValue:@NO forKey:@"drawsBackground"];

        NSString *target = [NSString stringWithUTF8String:url_or_path];
        if (is_local_file) {
            NSURL *fileURL = [NSURL fileURLWithPath:target];
            NSURL *readAccessDir = [fileURL URLByDeletingLastPathComponent];
            [webView loadFileURL:fileURL allowingReadAccessToURL:readAccessDir];
        } else {
            NSURL *url = [NSURL URLWithString:target];
            if (url) {
                [webView loadRequest:[NSURLRequest requestWithURL:url]];
            }
        }

        [root addSubview:webView];

        LivelyWebContext *ctx = (LivelyWebContext*)malloc(sizeof(LivelyWebContext));
        ctx->webView = webView;
        ctx->containerView = webView;

        CFRetain((__bridge CFTypeRef)webView);
        return ctx;
    }
}

void lively_web_eval_js(void* web_ptr, const char* js_code) {
    if (!web_ptr || !js_code) return;
    @autoreleasepool {
        LivelyWebContext *ctx = (LivelyWebContext*)web_ptr;
        NSString *code = [NSString stringWithUTF8String:js_code];
        [ctx->webView evaluateJavaScript:code completionHandler:nil];
    }
}

void lively_web_send_audio_spectrum(void* web_ptr, const float* bands, uint32_t count) {
    if (!web_ptr || !bands || count == 0) return;
    @autoreleasepool {
        LivelyWebContext *ctx = (LivelyWebContext*)web_ptr;
        NSMutableString *jsonArr = [NSMutableString stringWithString:@"["];
        for (uint32_t i = 0; i < count; i++) {
            if (i > 0) [jsonArr appendString:@","];
            [jsonArr appendFormat:@"%.4f", bands[i]];
        }
        [jsonArr appendString:@"]"];

        NSString *js = [NSString stringWithFormat:@"if(window.livelyAudioListener){ window.livelyAudioListener(%@); }", jsonArr];
        [ctx->webView evaluateJavaScript:js completionHandler:nil];
    }
}

void lively_web_send_system_data(void* web_ptr, const char* json_data) {
    if (!web_ptr || !json_data) return;
    @autoreleasepool {
        LivelyWebContext *ctx = (LivelyWebContext*)web_ptr;
        NSString *jsonStr = [NSString stringWithUTF8String:json_data];
        NSString *js = [NSString stringWithFormat:@"if(window.livelySystemInformation){ window.livelySystemInformation(%@); }", jsonStr];
        [ctx->webView evaluateJavaScript:js completionHandler:nil];
    }
}

void lively_web_send_property(void* web_ptr, const char* prop_name, const char* prop_value) {
    if (!web_ptr || !prop_name || !prop_value) return;
    @autoreleasepool {
        LivelyWebContext *ctx = (LivelyWebContext*)web_ptr;
        NSString *name = [NSString stringWithUTF8String:prop_name];
        NSString *val = [NSString stringWithUTF8String:prop_value];
        NSString *js = [NSString stringWithFormat:@"if(window.livelyPropertyListener){ window.livelyPropertyListener('%@', %@); }", name, val];
        [ctx->webView evaluateJavaScript:js completionHandler:nil];
    }
}

void lively_web_reload(void* web_ptr) {
    if (!web_ptr) return;
    @autoreleasepool {
        LivelyWebContext *ctx = (LivelyWebContext*)web_ptr;
        [ctx->webView reload];
    }
}

void lively_web_pause(void* web_ptr) {
    if (!web_ptr) return;
    @autoreleasepool {
        LivelyWebContext *ctx = (LivelyWebContext*)web_ptr;
        [ctx->webView evaluateJavaScript:@"if(window.livelyWallpaperPlaybackChanged) livelyWallpaperPlaybackChanged(false);" completionHandler:nil];
    }
}

void lively_web_resume(void* web_ptr) {
    if (!web_ptr) return;
    @autoreleasepool {
        LivelyWebContext *ctx = (LivelyWebContext*)web_ptr;
        [ctx->webView evaluateJavaScript:@"if(window.livelyWallpaperPlaybackChanged) livelyWallpaperPlaybackChanged(true);" completionHandler:nil];
    }
}

// A WKWebView keeps its content process alive until the view is fully torn
// down. Leaving the user scripts installed, or the view still in the hierarchy,
// retains it across wallpaper changes and the memory climbs by roughly a
// megabyte each time. Detach everything before releasing.
void lively_destroy_web_view(void* web_ptr) {
    if (!web_ptr) return;
    // Tearing down a WKWebView that is mid-load can raise; the handle is freed
    // regardless, because leaking it is worse than losing the exception.
    lively_guarded(^{ destroy_web_view_impl(web_ptr); return NULL; }, NULL);
}

static void destroy_web_view_impl(void* web_ptr) {
    @autoreleasepool {
        LivelyWebContext *ctx = (LivelyWebContext*)web_ptr;

        [ctx->webView stopLoading];
        [ctx->webView.configuration.userContentController removeAllUserScripts];
        ctx->webView.navigationDelegate = nil;
        [ctx->webView removeFromSuperview];

        if (ctx->webView) CFRelease((__bridge CFTypeRef)ctx->webView);
        free(ctx);
    }
}

#pragma mark - Metal Shader Engine

static const char* kDefaultVertexShader =
    "#include <metal_stdlib>\n"
    "using namespace metal;\n"
    "struct VertexOut {\n"
    "    float4 position [[position]];\n"
    "    float2 uv;\n"
    "};\n"
    "vertex VertexOut vertex_main(uint vid [[vertex_id]]) {\n"
    "    VertexOut out;\n"
    "    float2 quadVertices[4] = {\n"
    "        float2(-1.0, -1.0),\n"
    "        float2( 1.0, -1.0),\n"
    "        float2(-1.0,  1.0),\n"
    "        float2( 1.0,  1.0)\n"
    "    };\n"
    "    float2 pos = quadVertices[vid];\n"
    "    out.position = float4(pos, 0.0, 1.0);\n"
    "    out.uv = (pos + 1.0) * 0.5;\n"
    "    return out;\n"
    "}\n";

static const char* kDefaultFragmentShader =
    "#include <metal_stdlib>\n"
    "using namespace metal;\n"
    "struct ShaderUniforms {\n"
    "    float2 resolution;\n"
    "    float time;\n"
    "    float timeDelta;\n"
    "    int frame;\n"
    "    float4 mouse;\n"
    "    float audioBands[128];\n"
    "};\n"
    "struct VertexOut {\n"
    "    float4 position [[position]];\n"
    "    float2 uv;\n"
    "};\n"
    "fragment float4 fragment_main(VertexOut in [[stage_in]], constant ShaderUniforms &u [[buffer(0)]]) {\n"
    "    float2 uv = in.uv * 2.0 - 1.0;\n"
    "    uv.x *= u.resolution.x / u.resolution.y;\n"
    "    float d = length(uv);\n"
    "    float c = sin(d * 12.0 - u.time * 3.0);\n"
    "    float r = sin(u.time * 0.5) * 0.5 + 0.5;\n"
    "    float g = cos(u.time * 0.7) * 0.5 + 0.5;\n"
    "    float b = sin(u.time * 1.1) * 0.5 + 0.5;\n"
    "    return float4(c * r, c * g, c * b, 1.0);\n"
    "}\n";

void* lively_create_shader_view(void* win_ptr, const char* shader_source) {
    if (!win_ptr) return NULL;
    @autoreleasepool {
        NSWindow *win = (__bridge NSWindow*)win_ptr;
        id<MTLDevice> device = MTLCreateSystemDefaultDevice();
        if (!device) return NULL;

        NSView *root = [win contentView];
        MTKView *mtkView = [[MTKView alloc] initWithFrame:root.bounds device:device];
        mtkView.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        mtkView.colorPixelFormat = MTLPixelFormatBGRA8Unorm;
        mtkView.preferredFramesPerSecond = 60;

        LivelyShaderRenderer *renderer = [[LivelyShaderRenderer alloc] initWithDevice:device];

        NSString *src = nil;
        if (shader_source && strlen(shader_source) > 0) {
            src = [NSString stringWithUTF8String:shader_source];
        } else {
            src = [NSString stringWithFormat:@"%s\n%s", kDefaultVertexShader, kDefaultFragmentShader];
        }

        NSError *error = nil;
        id<MTLLibrary> library = [device newLibraryWithSource:src options:nil error:&error];
        if (!library) {
            NSLog(@"[Lively] Shader compile error: %@", error.localizedDescription);
            // Fallback to default
            src = [NSString stringWithFormat:@"%s\n%s", kDefaultVertexShader, kDefaultFragmentShader];
            library = [device newLibraryWithSource:src options:nil error:nil];
        }

        id<MTLFunction> vertFunc = [library newFunctionWithName:@"vertex_main"];
        id<MTLFunction> fragFunc = [library newFunctionWithName:@"fragment_main"];

        MTLRenderPipelineDescriptor *pDesc = [[MTLRenderPipelineDescriptor alloc] init];
        pDesc.vertexFunction = vertFunc;
        pDesc.fragmentFunction = fragFunc;
        pDesc.colorAttachments[0].pixelFormat = mtkView.colorPixelFormat;

        renderer.pipelineState = [device newRenderPipelineStateWithDescriptor:pDesc error:&error];
        mtkView.delegate = renderer;

        [root addSubview:mtkView];

        LivelyShaderContext *ctx = (LivelyShaderContext*)malloc(sizeof(LivelyShaderContext));
        ctx->mtkView = mtkView;
        ctx->renderer = renderer;

        CFRetain((__bridge CFTypeRef)mtkView);
        CFRetain((__bridge CFTypeRef)renderer);
        return ctx;
    }
}

void lively_shader_set_pause(void* shader_ptr, bool paused) {
    if (!shader_ptr) return;
    LivelyShaderContext *ctx = (LivelyShaderContext*)shader_ptr;
    ctx->renderer.paused = paused;
    ctx->mtkView.paused = paused;
}

void lively_shader_update_audio(void* shader_ptr, const float* bands, uint32_t count) {
    if (!shader_ptr || !bands || count == 0) return;
    LivelyShaderContext *ctx = (LivelyShaderContext*)shader_ptr;
    uint32_t toCopy = count < 128 ? count : 128;
    ctx->renderer.audioData = [NSData dataWithBytes:bands length:toCopy * sizeof(float)];
}

// Same ordering rule as the video view: break the ownership chain before
// dropping our explicit retains, otherwise the delegate and command queue stay
// reachable and each wallpaper switch leaks the renderer.
void lively_destroy_shader_view(void* shader_ptr) {
    if (!shader_ptr) return;
    @autoreleasepool {
        LivelyShaderContext *ctx = (LivelyShaderContext*)shader_ptr;

        ctx->mtkView.delegate = nil;
        [ctx->mtkView removeFromSuperview];

        if (ctx->mtkView)  CFRelease((__bridge CFTypeRef)ctx->mtkView);
        if (ctx->renderer) CFRelease((__bridge CFTypeRef)ctx->renderer);
        free(ctx);
    }
}

#pragma mark - Image / GIF Engine

// Point an existing image view at a different file.
//
// Decoding a 2560x1440 PNG produces a ~15 MB bitmap. Allocating a fresh view per
// wallpaper change drove RSS to hundreds of megabytes before the allocator
// reclaimed it; setting the image in place releases the previous bitmap
// immediately and keeps exactly one decoded copy live.
int lively_image_set_source(void* img_ptr, const char* file_path) {
    if (!img_ptr || !file_path) return 0;
    @autoreleasepool {
        LivelyImageContext *ctx = (LivelyImageContext*)img_ptr;

        NSString *path = [NSString stringWithUTF8String:file_path];
        NSImage *img = [[NSImage alloc] initWithContentsOfFile:path];
        if (!img) return 0;

        // Release the old retain only after the new bitmap is known good, so a
        // failed load leaves the current wallpaper untouched.
        CFRetain((__bridge CFTypeRef)img);
        NSImage *old = ctx->image;
        [ctx->imageView setImage:img];
        ctx->image = img;
        if (old) CFRelease((__bridge CFTypeRef)old);
        return 1;
    }
}

void* lively_create_image_view(void* win_ptr, const char* file_path, LivelyScaleMode scale_mode) {
    if (!win_ptr || !file_path) return NULL;
    @autoreleasepool {
        NSWindow *win = (__bridge NSWindow*)win_ptr;
        NSString *path = [NSString stringWithUTF8String:file_path];
        NSImage *img = [[NSImage alloc] initWithContentsOfFile:path];
        if (!img) return NULL;

        NSView *root = [win contentView];
        NSImageView *imgView = [[NSImageView alloc] initWithFrame:root.bounds];
        imgView.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        [imgView setImage:img];
        [imgView setAnimates:YES]; // Enables GIF animation automatically

        switch (scale_mode) {
            case SCALE_ASPECT_FILL:
                [imgView setImageScaling:NSImageScaleAxesIndependently];
                break;
            case SCALE_ASPECT_FIT:
                [imgView setImageScaling:NSImageScaleProportionallyUpOrDown];
                break;
            case SCALE_STRETCH:
            default:
                [imgView setImageScaling:NSImageScaleAxesIndependently];
                break;
        }

        [root addSubview:imgView];

        LivelyImageContext *ctx = (LivelyImageContext*)malloc(sizeof(LivelyImageContext));
        ctx->imageView = imgView;
        ctx->image = img;

        CFRetain((__bridge CFTypeRef)imgView);
        CFRetain((__bridge CFTypeRef)img);
        return ctx;
    }
}

void lively_destroy_image_view(void* img_ptr) {
    if (!img_ptr) return;
    @autoreleasepool {
        LivelyImageContext *ctx = (LivelyImageContext*)img_ptr;
        // Release the image before the view: the view's cell retains it, and
        // clearing it lets the bitmap's backing store go with it.
        ctx->imageView.image = nil;
        [ctx->imageView removeFromSuperview];
        if (ctx->imageView) CFRelease((__bridge CFTypeRef)ctx->imageView);
        if (ctx->image)     CFRelease((__bridge CFTypeRef)ctx->image);
        free(ctx);
    }
}

#pragma mark - Performance Automation

// THREAD-SAFE. Called from the automation watcher thread.
//
// Deliberately avoids AppKit (`NSScreen`) so this can run off the main thread:
// AppKit UI classes are not thread-safe and touching them off-thread is a crash
// risk. CoreGraphics display queries carry no such restriction.
bool lively_is_any_app_fullscreen(void) {
    @autoreleasepool {
        CGDirectDisplayID mainDisplay = CGMainDisplayID();

        // Compare against pixel dimensions: the window list reports pixel
        // bounds, and a fullscreen window covers the display exactly.
        size_t dispW = CGDisplayPixelsWide(mainDisplay);
        size_t dispH = CGDisplayPixelsHigh(mainDisplay);
        if (dispW == 0 || dispH == 0) return false;

        // Allow 1px slack: rounding between window bounds and display mode can
        // leave an exact-size window a pixel short, which would make us miss the
        // fullscreen transition entirely.
        CGFloat maxW = (CGFloat)dispW + 1.0;
        CGFloat maxH = (CGFloat)dispH + 1.0;

        CFArrayRef windowList = CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID);
        if (!windowList) return false;

        bool hasFullscreen = false;
        CFIndex count = CFArrayGetCount(windowList);
        for (CFIndex i = 0; i < count; i++) {
            CFDictionaryRef info = (CFDictionaryRef)CFArrayGetValueAtIndex(windowList, i);
            if (!info) continue;

            CFNumberRef layerRef = (CFNumberRef)CFDictionaryGetValue(info, kCGWindowLayer);
            int layer = 0;
            if (layerRef) CFNumberGetValue(layerRef, kCFNumberIntType, &layer);

            // Layer 0 is the normal application window layer. Anything else
            // (menus, tooltips, our own wallpaper windows) must not count.
            if (layer != 0) continue;

            CFDictionaryRef boundsDict = (CFDictionaryRef)CFDictionaryGetValue(info, kCGWindowBounds);
            CGRect bounds;
            if (boundsDict && CGRectMakeWithDictionaryRepresentation(boundsDict, &bounds)) {
                if (bounds.size.width >= maxW && bounds.size.height >= maxH) {
                    hasFullscreen = true;
                    break;
                }
            }
        }
        CFRelease(windowList);
        return hasFullscreen;
    }
}

// THREAD-SAFE. IOKit power-source APIs carry no main-thread requirement.
bool lively_is_running_on_battery(void) {
    CFTypeRef psInfo = IOPSCopyPowerSourcesInfo();
    if (!psInfo) return false;
    CFArrayRef psList = IOPSCopyPowerSourcesList(psInfo);
    if (!psList) {
        CFRelease(psInfo);
        return false;
    }

    bool onBattery = false;
    for (CFIndex i = 0; i < CFArrayGetCount(psList); i++) {
        CFDictionaryRef desc = IOPSGetPowerSourceDescription(psInfo, CFArrayGetValueAtIndex(psList, i));
        if (desc) {
            CFStringRef state = (CFStringRef)CFDictionaryGetValue(desc, CFSTR(kIOPSPowerSourceStateKey));
            if (state && CFStringCompare(state, CFSTR(kIOPSBatteryPowerValue), 0) == kCFCompareEqualTo) {
                onBattery = true;
                break;
            }
        }
    }

    CFRelease(psList);
    CFRelease(psInfo);
    return onBattery;
}

#pragma mark - Tray & Menu

@interface LivelyTrayTarget : NSObject
- (void)onMenuClicked:(NSMenuItem *)sender;
@end

@implementation LivelyTrayTarget
- (void)onMenuClicked:(NSMenuItem *)sender {
    if (g_tray_cb) {
        g_tray_cb((int)sender.tag);
    }
}
@end

static LivelyTrayTarget *g_tray_target = nil;

void lively_setup_tray_menu(TrayCallback callback) {
    @autoreleasepool {
        g_tray_cb = callback;
        g_tray_target = [[LivelyTrayTarget alloc] init];

        NSStatusBar *statusBar = [NSStatusBar systemStatusBar];
        g_status_item = [statusBar statusItemWithLength:NSVariableStatusItemLength];
        [g_status_item.button setTitle:@"⬡ Lively"];

        NSMenu *menu = [[NSMenu alloc] initWithTitle:@"Lively"];

        NSMenuItem *itemOpen = [[NSMenuItem alloc] initWithTitle:@"Open Lively..." action:@selector(onMenuClicked:) keyEquivalent:@"o"];
        itemOpen.tag = 1;
        itemOpen.target = g_tray_target;
        [menu addItem:itemOpen];

        [menu addItem:[NSMenuItem separatorItem]];

        NSMenuItem *itemPlayPause = [[NSMenuItem alloc] initWithTitle:@"Pause / Resume" action:@selector(onMenuClicked:) keyEquivalent:@"p"];
        itemPlayPause.tag = 2;
        itemPlayPause.target = g_tray_target;
        [menu addItem:itemPlayPause];

        NSMenuItem *itemMute = [[NSMenuItem alloc] initWithTitle:@"Mute / Unmute" action:@selector(onMenuClicked:) keyEquivalent:@"m"];
        itemMute.tag = 3;
        itemMute.target = g_tray_target;
        [menu addItem:itemMute];

        NSMenuItem *itemNext = [[NSMenuItem alloc] initWithTitle:@"Random Wallpaper" action:@selector(onMenuClicked:) keyEquivalent:@"r"];
        itemNext.tag = 4;
        itemNext.target = g_tray_target;
        [menu addItem:itemNext];

        [menu addItem:[NSMenuItem separatorItem]];

        NSMenuItem *itemQuit = [[NSMenuItem alloc] initWithTitle:@"Quit Lively" action:@selector(onMenuClicked:) keyEquivalent:@"q"];
        itemQuit.tag = 99;
        itemQuit.target = g_tray_target;
        [menu addItem:itemQuit];

        g_status_item.menu = menu;
    }
}

void lively_update_tray_title(const char* title) {
    if (!title || !g_status_item) return;
    @autoreleasepool {
        [g_status_item.button setTitle:[NSString stringWithUTF8String:title]];
    }
}

void lively_open_file_picker(char* out_path, uint32_t max_len) {
    if (!out_path || max_len == 0) return;
    @autoreleasepool {
        NSOpenPanel *panel = [NSOpenPanel openPanel];
        [panel setCanChooseFiles:YES];
        [panel setCanChooseDirectories:YES];
        [panel setAllowsMultipleSelection:NO];
        [panel setMessage:@"Select Wallpaper Media or Lively Project"];

        if ([panel runModal] == NSModalResponseOK) {
            NSURL *url = [[panel URLs] firstObject];
            if (url) {
                const char* cpath = [[url path] UTF8String];
                strncpy(out_path, cpath, max_len - 1);
                out_path[max_len - 1] = '\0';
                return;
            }
        }
        out_path[0] = '\0';
    }
}

#pragma mark - Screensaver

void* lively_create_screensaver_window(uint32_t screen_index) {
    @autoreleasepool {
        NSArray<NSScreen *> *screens = [NSScreen screens];
        if (screen_index >= [screens count]) return NULL;

        NSScreen *screen = screens[screen_index];
        NSRect frame = [screen frame];

        NSWindow *win = [[NSWindow alloc] initWithContentRect:frame
                                                    styleMask:NSWindowStyleMaskBorderless
                                                      backing:NSBackingStoreBuffered
                                                        defer:NO
                                                       screen:screen];
        [win setLevel:NSScreenSaverWindowLevel];
        [win setOpaque:YES];
        [win setBackgroundColor:[NSColor blackColor]];

        NSView *contentView = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, frame.size.width, frame.size.height)];
        [contentView setWantsLayer:YES];
        [win setContentView:contentView];

        [win orderFront:nil];
        return (__bridge_retained void*)win;
    }
}

void lively_close_screensaver_window(void* win_ptr) {
    if (!win_ptr) return;
    @autoreleasepool {
        NSWindow *win = (__bridge_transfer NSWindow*)win_ptr;
        [win orderOut:nil];
        [win close];
    }
}

#pragma mark - Main Dispatch & Helpers

void lively_dispatch_on_main(void (*fn)(void*), void* ctx) {
    if ([NSThread isMainThread]) {
        fn(ctx);
    } else {
        dispatch_async_f(dispatch_get_main_queue(), ctx, fn);
    }
}

bool lively_take_screenshot(void* win_ptr, const char* out_path) {
    if (!win_ptr || !out_path) return false;
    @autoreleasepool {
        NSWindow *win = (__bridge NSWindow*)win_ptr;
        NSView *view = [win contentView];
        if (!view) return false;

        NSBitmapImageRep *rep = [view bitmapImageRepForCachingDisplayInRect:[view bounds]];
        [view cacheDisplayInRect:[view bounds] toBitmapImageRep:rep];

        NSString *path = [NSString stringWithUTF8String:out_path];
        NSData *data = nil;
        if ([path hasSuffix:@".png"]) {
            data = [rep representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
        } else {
            data = [rep representationUsingType:NSBitmapImageFileTypeJPEG properties:@{NSImageCompressionFactor: @0.9}];
        }

        if (data) {
            return [data writeToFile:path atomically:YES];
        }
        return false;
    }
}

static id g_screensaver_monitor = nil;

void lively_start_screensaver_monitor(void (*dismiss_cb)(void)) {
    if (g_screensaver_monitor) return;
    g_screensaver_monitor = [NSEvent addGlobalMonitorForEventsMatchingMask:(NSEventMaskKeyDown | NSEventMaskLeftMouseDown | NSEventMaskRightMouseDown | NSEventMaskMouseMoved) handler:^(NSEvent * _Nonnull event) {
        (void)event;
        if (dismiss_cb) dismiss_cb();
    }];
}

void lively_stop_screensaver_monitor(void) {
    if (g_screensaver_monitor) {
        [NSEvent removeMonitor:g_screensaver_monitor];
        g_screensaver_monitor = nil;
    }
}
