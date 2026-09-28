package com.conduit.mobile

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.AccessibilityServiceInfo
import android.accessibilityservice.GestureDescription
import android.content.Context
import android.content.Intent
import android.graphics.Path
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.KeyEvent
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import org.json.JSONObject

class ConduitAccessibilityService : AccessibilityService() {
    companion object {
        private const val TAG = "ConduitAccessibility"
        private var instance: ConduitAccessibilityService? = null
        private const val REQUEST_CODE = 20001

        fun getInstance(): ConduitAccessibilityService? = instance

        /** Accessibility services must be enabled by the user in Settings —
         *  they cannot be started via startService(). This helper opens the
         *  system Accessibility Settings screen instead. */
        fun openAccessibilitySettings(context: android.content.Context) {
            try {
                val intent = Intent(android.provider.Settings.ACTION_ACCESSIBILITY_SETTINGS)
                intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                context.startActivity(intent)
            } catch (e: Exception) {
                Log.w(TAG, "Failed to open accessibility settings", e)
            }
        }

        @Deprecated("Use openAccessibilitySettings() — accessibility services cannot be started via startService()")
        fun startService(context: android.content.Context) {
            openAccessibilitySettings(context)
        }
    }

    private val handler = Handler(Looper.getMainLooper())

    override fun onServiceConnected() {
        super.onServiceConnected()
        instance = this
        // Flags are also declared in accessibility_service_config.xml; only
        // top-up here if the XML was not applied (defensive, keeps both in sync).
        try {
            val info = serviceInfo ?: AccessibilityServiceInfo()
            info.eventTypes = AccessibilityEvent.TYPES_ALL_MASK
            info.feedbackType = AccessibilityServiceInfo.FEEDBACK_GENERIC
            info.flags = info.flags or
                    AccessibilityServiceInfo.FLAG_INCLUDE_NOT_IMPORTANT_VIEWS or
                    AccessibilityServiceInfo.FLAG_REPORT_VIEW_IDS or
                    AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS
            info.notificationTimeout = 100
            serviceInfo = info
        } catch (e: Exception) {
            Log.w(TAG, "Failed to update serviceInfo", e)
        }

        Log.d(TAG, "Accessibility service connected")
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        // Not used for input injection
    }

    override fun onInterrupt() {
        Log.d(TAG, "Accessibility service interrupted")
    }

    override fun onDestroy() {
        super.onDestroy()
        instance = null
    }

    private fun screenSize(): Pair<Float, Float> {
        val metrics = resources.displayMetrics
        return Pair(metrics.widthPixels.toFloat(), metrics.heightPixels.toFloat())
    }

    fun injectTouch(x: Float, y: Float, actionType: String) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.N) {
            Log.w(TAG, "Gesture injection requires API 24+")
            return
        }

        val (displayWidth, displayHeight) = screenSize()

        // Desktop sends pixel coordinates; values strictly in 0..1 range are
        // treated as normalized fractions, anything larger is a raw pixel.
        val screenX = if (x in 0f..1f && y in 0f..1f) {
            (x * displayWidth).coerceIn(0f, displayWidth)
        } else {
            x.coerceIn(0f, displayWidth)
        }
        val screenY = if (x in 0f..1f && y in 0f..1f) {
            (y * displayHeight).coerceIn(0f, displayHeight)
        } else {
            y.coerceIn(0f, displayHeight)
        }

        val path = Path()
        path.moveTo(screenX, screenY)

        val gestureBuilder = GestureDescription.Builder()
        val strokeDescription = GestureDescription.StrokeDescription(path, 0, 100)
        gestureBuilder.addStroke(strokeDescription)

        val gesture = gestureBuilder.build()
        dispatchGesture(gesture, null, null)

        Log.d(TAG, "Touch injected: ($screenX, $screenY) type=$actionType")
    }

    fun injectKeyEvent(key: String, modifiers: List<String>) {
        val root = rootInActiveWindow
        if (root == null) {
            Log.w(TAG, "Key injection failed: no active window")
            return
        }
        val focus = root.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
            ?: root.findFocus(AccessibilityNodeInfo.FOCUS_ACCESSIBILITY)
        when (key) {
            "Enter" -> {
                // IME "enter": newline in editable fields, click otherwise.
                var acted = false
                if (focus != null && focus.isEditable) {
                    val args = Bundle()
                    args.putCharSequence(
                        AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                        "${focus.text ?: ""}\n"
                    )
                    acted = focus.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, args)
                } else {
                    acted = focus?.performAction(AccessibilityNodeInfo.ACTION_CLICK) == true
                }
                Log.d(TAG, "Enter injected (acted=$acted)")
            }
            "Backspace" -> {
                // Delete one char from an editable field when possible.
                var handled = false
                if (focus != null && focus.isEditable) {
                    val args = Bundle()
                    args.putCharSequence(
                        AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                        focus.text?.dropLast(1) ?: ""
                    )
                    handled = focus.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, args)
                }
                Log.d(TAG, "Backspace injected (handled=$handled)")
            }
            "Tab" -> {
                // No global focus-forward action exists; insert a tab char in
                // editable fields, otherwise move granularity within text.
                var acted = false
                if (focus != null && focus.isEditable) {
                    val args = Bundle()
                    args.putCharSequence(
                        AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                        "${focus.text ?: ""}\t"
                    )
                    acted = focus.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, args)
                }
                Log.d(TAG, "Tab injected (acted=$acted)")
            }
            "Escape" -> {
                performGlobalAction(GLOBAL_ACTION_BACK)
                Log.d(TAG, "Escape -> BACK injected")
            }
            "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End" -> {
                Log.w(TAG, "DPAD key $key has no accessibility equivalent; ignored")
            }
            " " -> insertText(focus, " ")
            else -> {
                if (key.length == 1) {
                    insertText(focus, key)
                } else {
                    Log.w(TAG, "Unsupported key: $key (modifiers=$modifiers)")
                }
            }
        }
        try { root.recycle() } catch (_: Exception) {}
    }

    private fun insertText(focus: AccessibilityNodeInfo?, text: String) {
        if (focus != null && focus.isEditable) {
            val args = Bundle()
            args.putCharSequence(
                AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                "${focus.text ?: ""}$text"
            )
            val ok = focus.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, args)
            Log.d(TAG, "Text '$text' injected (ok=$ok)")
        } else {
            Log.w(TAG, "No editable focus for text '$text'")
        }
    }

    fun injectScroll(delta: Float) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.N) return

        // Scroll gesture — swipe up or down around screen center
        val (displayWidth, displayHeight) = screenSize()
        val centerX = displayWidth / 2f
        val startY = displayHeight / 2f
        val endY = startY - delta * 10

        val path = Path()
        path.moveTo(centerX, startY)
        path.lineTo(centerX, endY)

        val gestureBuilder = GestureDescription.Builder()
        gestureBuilder.addStroke(GestureDescription.StrokeDescription(path, 0, 300))
        dispatchGesture(gestureBuilder.build(), null, null)

        Log.d(TAG, "Scroll injected: delta=$delta")
    }

    fun injectMouseMove(x: Float, y: Float) {
        // Mouse move on Android — move accessibility focus or scroll position
        injectTouch(x, y, "move")
    }

    fun injectMouseClick(x: Float, y: Float, button: String) {
        injectTouch(x, y, "tap")
    }
}
