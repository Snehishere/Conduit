package com.conduit.mobile

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.PixelFormat
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.Image
import android.media.ImageReader
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Base64
import android.util.DisplayMetrics
import android.util.Log
import android.view.WindowManager
import androidx.core.app.ActivityCompat
import io.flutter.plugin.common.EventChannel
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer

class ScreenMirrorService(
    private val context: Context,
    private val eventChannel: EventChannel.EventSink?
) {
    companion object {
        private const val TAG = "ScreenMirrorService"
        private const val REQUEST_MEDIA_PROJECTION = 10001
    }

    private var mediaProjection: MediaProjection? = null
    private var virtualDisplay: VirtualDisplay? = null
    private var imageReader: ImageReader? = null
    private val handler = Handler(Looper.getMainLooper())
    @Volatile private var isStreaming = false
    private var streamJob: java.util.concurrent.ScheduledExecutorService? = null
    private var jpegQuality = 80

    fun startCapture(data: Intent, deviceId: String, quality: String, fps: Int) {
        // Stop any previous session first (idempotent).
        stopCapture()
        try {
            val projectionManager = context.getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
            mediaProjection = projectionManager.getMediaProjection(Activity.RESULT_OK, data)

            val wm = context.getSystemService(Context.WINDOW_SERVICE) as WindowManager
            val metrics = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                val wm2 = context.getSystemService(WindowManager::class.java)
                val bounds = wm2.currentWindowMetrics.bounds
                DisplayMetrics().also {
                    it.widthPixels = bounds.width()
                    it.heightPixels = bounds.height()
                    it.densityDpi = context.resources.displayMetrics.densityDpi
                }
            } else {
                DisplayMetrics().also {
                    @Suppress("DEPRECATION")
                    wm.defaultDisplay.getRealMetrics(it)
                }
            }

            // Scale based on quality
            val scale = when (quality) {
                "low" -> 0.25f
                "medium" -> 0.5f
                "high" -> 0.75f
                else -> 0.5f
            }
            jpegQuality = when (quality) {
                "low" -> 60
                "medium" -> 80
                "high" -> 90
                else -> 80
            }
            val safeFps = fps.coerceIn(1, 30)
            val width = (metrics.widthPixels * scale).toInt().coerceAtLeast(2)
            val height = (metrics.heightPixels * scale).toInt().coerceAtLeast(2)
            val density = metrics.densityDpi

            imageReader = ImageReader.newInstance(width, height, PixelFormat.RGBA_8888, 2)

            virtualDisplay = mediaProjection?.createVirtualDisplay(
                "Conduit Mirror",
                width, height, density,
                DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
                imageReader?.surface,
                null, handler
            )

            isStreaming = true

            // Capture frames at specified FPS (clamped 1..30)
            val interval = 1000L / safeFps
            streamJob = java.util.concurrent.Executors.newSingleThreadScheduledExecutor()
            streamJob?.scheduleAtFixedRate({
                if (isStreaming) {
                    captureFrame(deviceId, width, height)
                }
            }, 0, interval, java.util.concurrent.TimeUnit.MILLISECONDS)

            Log.d(TAG, "Screen mirroring started: ${width}x${height} @ ${safeFps}fps (q=$quality)")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to start screen mirroring", e)
        }
    }

    private fun captureFrame(deviceId: String, width: Int, height: Int) {
        // Snapshot reader reference to avoid race with stopCapture() nulling it.
        val reader = imageReader ?: return
        var image: Image? = null
        try {
            image = try {
                reader.acquireLatestImage()
            } catch (e: IllegalStateException) {
                // Reader closed concurrently by stopCapture()
                return
            }
            if (image != null) {
                val planes = image.planes
                val buffer: ByteBuffer = planes[0].buffer
                val pixelStride = planes[0].pixelStride
                val rowStride = planes[0].rowStride
                val rowPadding = rowStride - pixelStride * width

                val bitmap = Bitmap.createBitmap(
                    width + rowPadding / pixelStride,
                    height,
                    Bitmap.Config.ARGB_8888
                )
                bitmap.copyPixelsFromBuffer(buffer)

                // Crop to actual size if needed
                val croppedBitmap = if (bitmap.width != width || bitmap.height != height) {
                    Bitmap.createBitmap(bitmap, 0, 0, width, height).also { bitmap.recycle() }
                } else {
                    bitmap
                }

                // Encode to JPEG (quality follows requested preset)
                val outputStream = ByteArrayOutputStream()
                croppedBitmap.compress(Bitmap.CompressFormat.JPEG, jpegQuality, outputStream)
                val base64 = Base64.encodeToString(outputStream.toByteArray(), Base64.NO_WRAP)
                croppedBitmap.recycle()

                // Send frame to Flutter via EventChannel (Flutter relays to WS).
                // Flutter must subscribe with EventChannel('com.conduit.mobile/events'), arg 'screen_mirror'.
                val frame = JSONObject()
                frame.put("type", "screen_mirror")
                frame.put("action", "frame")
                frame.put("device_id", deviceId)
                frame.put("data", base64)
                frame.put("width", width)
                frame.put("height", height)
                frame.put("timestamp", System.currentTimeMillis() / 1000)

                handler.post {
                    try {
                        eventChannel?.success(jsonToMap(frame))
                    } catch (e: Exception) {
                        Log.w(TAG, "EventChannel send failed", e)
                    }
                }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to capture frame", e)
        } finally {
            try { image?.close() } catch (_: Exception) {}
        }
    }

    private fun jsonToMap(obj: JSONObject): Map<String, Any?> {
        val map = mutableMapOf<String, Any?>()
        val keys = obj.keys()
        while (keys.hasNext()) {
            val key = keys.next()
            map[key] = obj.opt(key)
        }
        return map
    }

    fun stopCapture() {
        isStreaming = false
        try {
            streamJob?.shutdownNow()
            // Brief grace so an in-flight captureFrame() can exit before teardown.
            streamJob?.awaitTermination(300, java.util.concurrent.TimeUnit.MILLISECONDS)
        } catch (_: Exception) {}
        streamJob = null
        try { virtualDisplay?.release() } catch (_: Exception) {}
        virtualDisplay = null
        try { imageReader?.close() } catch (_: Exception) {}
        imageReader = null
        try { mediaProjection?.stop() } catch (_: Exception) {}
        mediaProjection = null
        Log.d(TAG, "Screen mirroring stopped")
    }

    fun injectTouchEvent(x: Float, y: Float, actionType: String) {
        // Touch injection requires AccessibilityService — handled there
        Log.d(TAG, "Touch event: ($x, $y) type=$actionType")
    }

    fun injectKeyEvent(key: String, modifiers: List<String>) {
        // Key injection requires AccessibilityService — handled there
        Log.d(TAG, "Key event: $key modifiers=$modifiers")
    }
}
