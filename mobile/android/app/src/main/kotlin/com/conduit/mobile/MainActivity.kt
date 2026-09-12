package com.conduit.mobile

import android.Manifest
import android.app.Activity
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.media.projection.MediaProjectionManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.ContactsContract
import android.provider.Telephony
import android.telephony.PhoneStateListener
import android.telephony.SmsManager
import android.telephony.TelephonyManager
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.EventChannel
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import org.json.JSONArray
import org.json.JSONObject

class MainActivity : FlutterActivity() {
    private val CHANNEL = "com.conduit.mobile/native"
    private val EVENT_CHANNEL = "com.conduit.mobile/events"
    private val SCREEN_MIRROR_CHANNEL = "com.conduit.mobile/screen_mirror"
    private val REMOTE_INPUT_CHANNEL = "com.conduit.mobile/remote_input"
    private val PERMISSION_REQUEST_CODE = 1001
    private val MEDIA_PROJECTION_REQUEST = 10001

    private var notificationEventSink: EventChannel.EventSink? = null
    private var callEventSink: EventChannel.EventSink? = null
    private var screenMirrorEventSink: EventChannel.EventSink? = null
    private var remoteInputEventSink: EventChannel.EventSink? = null
    private val handler = Handler(Looper.getMainLooper())

    private var telephonyManager: TelephonyManager? = null
    private var phoneStateListener: PhoneStateListener? = null
    private var screenMirrorService: ScreenMirrorService? = null
    private var notificationReceiver: BroadcastReceiver? = null

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)

        // Request permissions on startup
        requestPermissions()

        // Method Channel — synchronous calls from Flutter
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, CHANNEL).setMethodCallHandler { call, result ->
            when (call.method) {
                "getNotifications" -> getNotifications(result)
                "getSmsThreads" -> getSmsThreads(result)
                "sendSms" -> {
                    val to = call.argument<String>("to") ?: ""
                    val body = call.argument<String>("body") ?: ""
                    sendSms(to, body, result)
                }
                "getCallLog" -> getCallLog(result)
                "getDeviceInfo" -> getDeviceInfo(result)
                "getBatteryLevel" -> getBatteryLevel(result)
                "isNotificationListenerEnabled" -> isNotificationListenerEnabled(result)
                "openNotificationListenerSettings" -> openNotificationListenerSettings(result)
                "getSmsPermissions" -> getSmsPermissions(result)
                "getPhoneState" -> getPhoneState(result)
                "getBluetoothDevices" -> getBluetoothDevices(result)
                "startScreenMirror" -> startScreenMirror(call, result)
                "stopScreenMirror" -> stopScreenMirror(result)
                "isAccessibilityServiceEnabled" -> isAccessibilityServiceEnabled(result)
                "openAccessibilitySettings" -> openAccessibilitySettings(result)
                "getWifiName" -> getWifiName(result)
                "getRunningApps" -> getRunningApps(result)
                "injectTouch" -> {
                    val x = call.argument<Double>("x")?.toFloat() ?: 0f
                    val y = call.argument<Double>("y")?.toFloat() ?: 0f
                    val actionType = call.argument<String>("actionType") ?: "tap"
                    injectTouch(x, y, actionType, result)
                }
                "injectKey" -> {
                    val key = call.argument<String>("key") ?: ""
                    val modifiers = call.argument<List<String>>("modifiers") ?: emptyList()
                    injectKey(key, modifiers, result)
                }
                "injectScroll" -> {
                    val delta = call.argument<Double>("delta")?.toFloat() ?: 0f
                    injectScroll(delta, result)
                }
                else -> result.notImplemented()
            }
        }

        // Event Channel — streaming events (notifications, calls, screen mirror, remote input)
        EventChannel(flutterEngine.dartExecutor.binaryMessenger, EVENT_CHANNEL).setStreamHandler(
            object : EventChannel.StreamHandler {
                override fun onListen(arguments: Any?, events: EventChannel.EventSink?) {
                    val type = arguments as? String
                    when (type) {
                        "notifications" -> notificationEventSink = events
                        "calls" -> callEventSink = events
                        "screen_mirror" -> screenMirrorEventSink = events
                        "remote_input" -> remoteInputEventSink = events
                    }
                    startListening()
                }

                override fun onCancel(arguments: Any?) {
                    // Only tear down the stream that was actually cancelled.
                    when (arguments as? String) {
                        "notifications" -> notificationEventSink = null
                        "calls" -> callEventSink = null
                        "screen_mirror" -> screenMirrorEventSink = null
                        "remote_input" -> remoteInputEventSink = null
                        else -> {
                            // Unknown — do not nuke all sinks; log only.
                            android.util.Log.w("MainActivity", "EventChannel onCancel with unknown args: $arguments")
                        }
                    }
                }
            }
        )
    }

    private fun requestPermissions() {
        val permissions = mutableListOf<String>()
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.READ_SMS) != PackageManager.PERMISSION_GRANTED) {
            permissions.add(Manifest.permission.READ_SMS)
        }
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.SEND_SMS) != PackageManager.PERMISSION_GRANTED) {
            permissions.add(Manifest.permission.SEND_SMS)
        }
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.READ_PHONE_STATE) != PackageManager.PERMISSION_GRANTED) {
            permissions.add(Manifest.permission.READ_PHONE_STATE)
        }
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.READ_CALL_LOG) != PackageManager.PERMISSION_GRANTED) {
            permissions.add(Manifest.permission.READ_CALL_LOG)
        }
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                permissions.add(Manifest.permission.POST_NOTIFICATIONS)
            }
        }
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                permissions.add(Manifest.permission.BLUETOOTH_CONNECT)
            }
        }
        if (permissions.isNotEmpty()) {
            ActivityCompat.requestPermissions(this, permissions.toTypedArray(), PERMISSION_REQUEST_CODE)
        }
    }

    private var listeningStarted = false

    private fun startListening() {
        // Guard against double-registration: each onListen call re-invokes this.
        if (listeningStarted) return
        listeningStarted = true
        // Start phone state listener for calls
        startPhoneStateListener()

        // Register broadcast receiver for notifications from NotificationListenerService
        notificationReceiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context?, intent: Intent?) {
                val eventStr = intent?.getStringExtra("event") ?: return
                try {
                    val event = JSONObject(eventStr)
                    val eventMap = mutableMapOf<String, Any>()
                    event.keys().forEach { key ->
                        eventMap[key] = event.get(key)
                    }
                    handler.post { notificationEventSink?.success(eventMap) }
                } catch (e: Exception) {
                    android.util.Log.e("MainActivity", "Error forwarding notification", e)
                }
            }
        }
        val filter = IntentFilter("com.conduit.mobile.NOTIFICATION_EVENT")
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            registerReceiver(notificationReceiver, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            try {
                androidx.core.content.ContextCompat.registerReceiver(
                    this, notificationReceiver, filter, androidx.core.content.ContextCompat.RECEIVER_NOT_EXPORTED
                )
            } catch (e: Exception) {
                // Fallback for old core versions without RECEIVER_NOT_EXPORTED
                registerReceiver(notificationReceiver, filter)
            }
        }
    }

    private fun stopListening() {
        listeningStarted = false
        notificationReceiver?.let {
            try { unregisterReceiver(it) } catch (e: IllegalArgumentException) {
                android.util.Log.w("MainActivity", "Receiver already unregistered", e)
            }
            notificationReceiver = null
        }
        try {
            @Suppress("DEPRECATION")
            telephonyManager?.listen(phoneStateListener, PhoneStateListener.LISTEN_NONE)
        } catch (e: Exception) {
            android.util.Log.w("MainActivity", "Failed to unregister phone listener", e)
        }
        phoneStateListener = null
    }

    private fun startPhoneStateListener() {
        telephonyManager = getSystemService(Context.TELEPHONY_SERVICE) as? TelephonyManager
        if (ActivityCompat.checkSelfPermission(this, Manifest.permission.READ_PHONE_STATE) == PackageManager.PERMISSION_GRANTED) {
            @Suppress("DEPRECATION")
            phoneStateListener = object : PhoneStateListener() {
                @Deprecated("Deprecated in API 31")
                override fun onCallStateChanged(state: Int, phoneNumber: String?) {
                    super.onCallStateChanged(state, phoneNumber)
                    val event = JSONObject()
                    event.put("type", "call_state")
                    when (state) {
                        TelephonyManager.CALL_STATE_RINGING -> {
                            event.put("state", "ringing")
                            event.put("number", phoneNumber ?: "unknown")
                        }
                        TelephonyManager.CALL_STATE_OFFHOOK -> {
                            event.put("state", "active")
                        }
                        TelephonyManager.CALL_STATE_IDLE -> {
                            event.put("state", "idle")
                        }
                    }
                    handler.post { callEventSink?.success(jsonToMap(event)) }
                }
            }
            telephonyManager?.listen(phoneStateListener, PhoneStateListener.LISTEN_CALL_STATE)
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

    // --- Method Channel Handlers ---

    private fun getNotifications(result: MethodChannel.Result) {
        val listener = ConduitNotificationListener.getInstance()
        if (listener == null) {
            result.success(JSONArray().toString())
            return
        }

        val notifications = JSONArray()
        try {
            val activeNotifications = listener.activeNotifications
            if (activeNotifications != null) {
                for (sbn in activeNotifications) {
                    val notification = sbn.notification
                    val extras = notification?.extras
                    val obj = JSONObject()
                    obj.put("id", sbn.key)
                    obj.put("app", sbn.packageName ?: "unknown")
                    obj.put("title", extras?.getCharSequence(android.app.Notification.EXTRA_TITLE)?.toString() ?: "")
                    obj.put("body", extras?.getCharSequence(android.app.Notification.EXTRA_TEXT)?.toString() ?: "")
                    obj.put("timestamp", sbn.postTime / 1000)
                    notifications.put(obj)
                }
            }
        } catch (e: Exception) {
            android.util.Log.e("MainActivity", "Error getting notifications", e)
        }
        result.success(notifications.toString())
    }

    private fun getSmsThreads(result: MethodChannel.Result) {
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.READ_SMS) != PackageManager.PERMISSION_GRANTED) {
            result.error("PERMISSION_DENIED", "SMS permission not granted", null)
            return
        }

        val threads = JSONArray()
        val cursor = contentResolver.query(
            Uri.parse("content://sms"),
            arrayOf("_id", "address", "body", "date", "type", "read"),
            null, null,
            "date DESC"
        )

        cursor?.use {
            val addressIdx = it.getColumnIndex("address")
            val bodyIdx = it.getColumnIndex("body")
            val dateIdx = it.getColumnIndex("date")
            val typeIdx = it.getColumnIndex("type")
            val readIdx = it.getColumnIndex("read")

            while (it.moveToNext()) {
                val thread = JSONObject()
                thread.put("address", it.getString(addressIdx) ?: "")
                thread.put("body", it.getString(bodyIdx) ?: "")
                thread.put("timestamp", (it.getLong(dateIdx) / 1000).toInt())
                thread.put("is_outgoing", it.getInt(typeIdx) == 2)
                thread.put("read", it.getInt(readIdx) == 1)
                threads.put(thread)
            }
        }

        result.success(threads.toString())
    }

    private fun sendSms(to: String, body: String, result: MethodChannel.Result) {
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.SEND_SMS) != PackageManager.PERMISSION_GRANTED) {
            result.error("PERMISSION_DENIED", "SMS permission not granted", null)
            return
        }

        try {
            val smsManager = SmsManager.getDefault()
            smsManager.sendTextMessage(to, null, body, null, null)
            result.success(true)
        } catch (e: Exception) {
            result.error("SMS_FAILED", e.message, null)
        }
    }

    private fun getCallLog(result: MethodChannel.Result) {
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.READ_CALL_LOG) != PackageManager.PERMISSION_GRANTED) {
            result.error("PERMISSION_DENIED", "Call log permission not granted", null)
            return
        }

        val calls = JSONArray()
        val cursor = contentResolver.query(
            android.provider.CallLog.Calls.CONTENT_URI,
            arrayOf("number", "date", "type", "duration"),
            null, null,
            "date DESC"
        )

        cursor?.use {
            val numberIdx = it.getColumnIndex("number")
            val dateIdx = it.getColumnIndex("date")
            val typeIdx = it.getColumnIndex("type")
            val durationIdx = it.getColumnIndex("duration")

            while (it.moveToNext()) {
                val call = JSONObject()
                call.put("number", it.getString(numberIdx) ?: "")
                call.put("timestamp", (it.getLong(dateIdx) / 1000).toInt())
                call.put("type", when (it.getInt(typeIdx)) {
                    1 -> "incoming"
                    2 -> "outgoing"
                    3 -> "missed"
                    else -> "unknown"
                })
                call.put("duration", it.getInt(durationIdx))
                calls.put(call)
            }
        }

        result.success(calls.toString())
    }

    private fun getDeviceInfo(result: MethodChannel.Result) {
        val info = JSONObject()
        info.put("device_name", Build.MANUFACTURER + " " + Build.MODEL)
        info.put("os", "Android ${Build.VERSION.RELEASE}")
        info.put("sdk_version", Build.VERSION.SDK_INT)
        info.put("device_id", Build.SERIAL ?: "unknown")
        result.success(info.toString())
    }

    private fun getBatteryLevel(result: MethodChannel.Result) {
        val batteryManager = getSystemService(Context.BATTERY_SERVICE) as android.os.BatteryManager
        val level = batteryManager.getIntProperty(android.os.BatteryManager.BATTERY_PROPERTY_CAPACITY)
        result.success(level)
    }

    private fun isNotificationListenerEnabled(result: MethodChannel.Result) {
        val enabled = try {
            val flat = android.provider.Settings.Secure.getString(contentResolver, "enabled_notification_listeners")
            flat?.contains(packageName) == true
        } catch (e: Exception) {
            false
        }
        result.success(enabled)
    }

    private fun openNotificationListenerSettings(result: MethodChannel.Result) {
        try {
            startActivity(Intent("android.settings.ACTION_NOTIFICATION_LISTENER_SETTINGS"))
            result.success(true)
        } catch (e: Exception) {
            result.error("LAUNCH_FAILED", e.message, null)
        }
    }

    private fun getSmsPermissions(result: MethodChannel.Result) {
        val granted = ContextCompat.checkSelfPermission(this, Manifest.permission.READ_SMS) == PackageManager.PERMISSION_GRANTED
        result.success(granted)
    }

    private fun getPhoneState(result: MethodChannel.Result) {
        val state = JSONObject()
        state.put("has_phone", telephonyManager?.phoneType != TelephonyManager.PHONE_TYPE_NONE)
        result.success(state.toString())
    }

    private fun getBluetoothDevices(result: MethodChannel.Result) {
        val devices = JSONArray()
        try {
            val bluetoothManager = getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager
            val adapter = bluetoothManager?.adapter
            if (adapter != null && adapter.isEnabled) {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                    if (ContextCompat.checkSelfPermission(this, Manifest.permission.BLUETOOTH_CONNECT) == PackageManager.PERMISSION_GRANTED) {
                        adapter.bondedDevices?.forEach { device ->
                            val d = JSONObject()
                            d.put("name", device.name ?: "Unknown")
                            d.put("address", device.address)
                            d.put("type", device.type)
                            devices.put(d)
                        }
                    }
                } else {
                    @Suppress("DEPRECATION")
                    adapter.bondedDevices?.forEach { device ->
                        val d = JSONObject()
                        d.put("name", device.name ?: "Unknown")
                        d.put("address", device.address)
                        d.put("type", device.type)
                        devices.put(d)
                    }
                }
            }
        } catch (_: Exception) {}
        result.success(devices.toString())
    }

    // --- Screen Mirroring ---

    private var pendingMirrorResult: MethodChannel.Result? = null
    private var pendingMirrorQuality: String = "medium"
    private var pendingMirrorFps: Int = 15

    private fun startScreenMirror(call: MethodCall, result: MethodChannel.Result) {
        pendingMirrorQuality = call.argument<String>("quality") ?: "medium"
        pendingMirrorFps = (call.argument<Int>("fps") ?: 15).coerceIn(1, 30)

        val projectionManager = getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        // Hold the result until onActivityResult tells us whether the user granted capture.
        pendingMirrorResult = result
        try {
            startActivityForResult(projectionManager.createScreenCaptureIntent(), MEDIA_PROJECTION_REQUEST)
        } catch (e: Exception) {
            pendingMirrorResult = null
            result.error("LAUNCH_FAILED", e.message, null)
        }
    }

    private fun stopScreenMirror(result: MethodChannel.Result) {
        screenMirrorService?.stopCapture()
        screenMirrorService = null
        result.success(true)
    }

    private fun injectTouch(x: Float, y: Float, actionType: String, result: MethodChannel.Result) {
        val accessibilityService = ConduitAccessibilityService.getInstance()
        if (accessibilityService != null) {
            accessibilityService.injectTouch(x, y, actionType)
            result.success(true)
        } else {
            result.error("NO_ACCESSIBILITY", "Accessibility service not enabled", null)
        }
    }

    private fun injectKey(key: String, modifiers: List<String>, result: MethodChannel.Result) {
        val accessibilityService = ConduitAccessibilityService.getInstance()
        if (accessibilityService != null) {
            accessibilityService.injectKeyEvent(key, modifiers)
            result.success(true)
        } else {
            result.error("NO_ACCESSIBILITY", "Accessibility service not enabled", null)
        }
    }

    private fun injectScroll(delta: Float, result: MethodChannel.Result) {
        val accessibilityService = ConduitAccessibilityService.getInstance()
        if (accessibilityService != null) {
            accessibilityService.injectScroll(delta)
            result.success(true)
        } else {
            result.error("NO_ACCESSIBILITY", "Accessibility service not enabled", null)
        }
    }

    private fun isAccessibilityServiceEnabled(result: MethodChannel.Result) {
        val enabled = ConduitAccessibilityService.getInstance() != null
        result.success(enabled)
    }

    private fun openAccessibilitySettings(result: MethodChannel.Result) {
        try {
            startActivity(Intent(android.provider.Settings.ACTION_ACCESSIBILITY_SETTINGS))
            result.success(true)
        } catch (e: Exception) {
            result.error("LAUNCH_FAILED", e.message, null)
        }
    }

    private fun getWifiName(result: MethodChannel.Result) {
        try {
            val wifiManager = applicationContext.getSystemService(Context.WIFI_SERVICE) as android.net.wifi.WifiManager
            val wifiInfo = wifiManager.connectionInfo
            val ssid = wifiInfo?.ssid?.replace("\"", "") ?: "unknown"
            result.success(ssid)
        } catch (e: Exception) {
            result.success("unknown")
        }
    }

    private fun getRunningApps(result: MethodChannel.Result) {
        try {
            val activityManager = getSystemService(Context.ACTIVITY_SERVICE) as android.app.ActivityManager
            val runningProcesses = activityManager.runningAppProcesses ?: emptyList()
            val apps = JSONArray()
            for (process in runningProcesses) {
                if (process.importance == android.app.ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND) {
                    for (pkg in process.pkgList) {
                        apps.put(pkg)
                    }
                }
            }
            result.success(apps.toString())
        } catch (e: Exception) {
            result.success("[]")
        }
    }

    // Handle MediaProjection result
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == MEDIA_PROJECTION_REQUEST) {
            val pending = pendingMirrorResult
            pendingMirrorResult = null
            if (resultCode == Activity.RESULT_OK && data != null) {
                screenMirrorService = ScreenMirrorService(this, screenMirrorEventSink)
                screenMirrorService?.startCapture(data, "phone", pendingMirrorQuality, pendingMirrorFps)
                pending?.success(true)
            } else {
                pending?.error("DENIED", "User denied screen capture permission", null)
            }
        }
    }

    override fun onDestroy() {
        stopListening()
        try {
            screenMirrorService?.stopCapture()
        } catch (e: Exception) {
            android.util.Log.w("MainActivity", "stopCapture failed", e)
        }
        super.onDestroy()
    }
}
