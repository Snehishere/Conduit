package com.conduit.mobile

import android.app.Notification
import android.content.Intent
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Log
import org.json.JSONObject

class ConduitNotificationListener : NotificationListenerService() {

    companion object {
        private const val TAG = "ConduitNotifListener"
        private var instance: ConduitNotificationListener? = null

        fun getInstance(): ConduitNotificationListener? = instance
    }

    override fun onCreate() {
        super.onCreate()
        instance = this
        Log.d(TAG, "Notification listener created")
    }

    override fun onDestroy() {
        super.onDestroy()
        instance = null
    }

    override fun onNotificationPosted(sbn: StatusBarNotification) {
        try {
            val notification = sbn.notification
            val extras = notification?.extras

            val event = JSONObject()
            event.put("type", "notification")
            event.put("action", "post")
            event.put("id", sbn.key)
            event.put("app", sbn.packageName ?: "unknown")
            event.put("title", extras?.getCharSequence(Notification.EXTRA_TITLE)?.toString() ?: "")
            event.put("body", extras?.getCharSequence(Notification.EXTRA_TEXT)?.toString() ?: "")
            event.put("timestamp", sbn.postTime / 1000)

            // Broadcast to Flutter via method channel or event
            sendBroadcast(Intent("com.conduit.mobile.NOTIFICATION_EVENT").apply {
                putExtra("event", event.toString())
                setPackage(packageName)
            })
        } catch (e: Exception) {
            Log.e(TAG, "Error processing notification", e)
        }
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification) {
        try {
            val event = JSONObject()
            event.put("type", "notification")
            event.put("action", "dismiss")
            event.put("id", sbn.key)

            sendBroadcast(Intent("com.conduit.mobile.NOTIFICATION_EVENT").apply {
                putExtra("event", event.toString())
                setPackage(packageName)
            })
        } catch (e: Exception) {
            Log.e(TAG, "Error processing notification removal", e)
        }
    }
}
