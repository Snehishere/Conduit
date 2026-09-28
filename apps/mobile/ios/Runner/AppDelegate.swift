import UIKit
import Flutter
import UserNotifications
import CoreBluetooth
import CallKit
import NetworkExtension
import ReplayKit

// MARK: - FlutterStreamHandler

/// A unified stream handler that routes events by the argument passed from
/// `EventChannel.receiveBroadcastStream(arguments)`.
///
/// Each call to `receiveBroadcastStream('notifications')`, `('calls')`, etc.
/// triggers a separate `onListen` invocation with the corresponding argument.
class ConduitStreamHandler: NSObject, FlutterStreamHandler {
    /// Maps argument strings to their active EventSinks.
    private var sinks: [String: FlutterEventSink] = [:]
    private let lock = NSLock()

    func onListen(arguments: Any?, events: @escaping FlutterEventSink?) -> FlutterError? {
        guard let type = arguments as? String else {
            return FlutterError(code: "INVALID_ARGS",
                                message: "Stream argument must be a string",
                                details: nil)
        }
        lock.lock()
        sinks[type] = events
        lock.unlock()
        return nil
    }

    func onCancel(arguments: Any?) -> FlutterError? {
        guard let type = arguments as? String else { return nil }
        lock.lock()
        sinks.removeValue(forKey: type)
        lock.unlock()
        return nil
    }

    /// Send an event to a specific stream type.
    func send(type: String, event: Any) {
        lock.lock()
        let sink = sinks[type]
        lock.unlock()
        DispatchQueue.main.async {
            sink?(event)
        }
    }

    /// Convenience: send a dictionary event.
    func send(type: String, data: [String: Any]) {
        send(type: type, event: data)
    }
}

// MARK: - AppDelegate

@main
@objc class AppDelegate: FlutterAppDelegate, CBCentralManagerDelegate, CXCallObserverDelegate {
    private var apnsToken: String?
    private var centralManager: CBCentralManager!
    private let callObserver = CXCallObserver()
    private var lastCallState: [String: Any] = ["state": "idle"]

    // Event channels
    private var mainStreamHandler: ConduitStreamHandler!
    private var smsStreamHandler: ConduitStreamHandler!

    // Screen mirror state
    private var isScreenCapturing = false
    private var screenCaptureTimer: Timer?

    override func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
    ) -> Bool {
        let controller: FlutterViewController = window?.rootViewController as! FlutterViewController
        let messenger = controller.binaryMessenger

        // ── Stream Handlers ──────────────────────────────────────────────
        mainStreamHandler = ConduitStreamHandler()
        smsStreamHandler   = ConduitStreamHandler()

        // ── Method Channel ───────────────────────────────────────────────
        let nativeChannel = FlutterMethodChannel(name: "com.conduit.mobile/native", binaryMessenger: messenger)

        centralManager = CBCentralManager(delegate: self, queue: nil)
        callObserver.setDelegate(self, queue: nil)

        nativeChannel.setMethodCallHandler { [weak self] (call: FlutterMethodCall, result: @escaping FlutterResult) in
            switch call.method {
            // ── Battery & Device ──
            case "getBatteryLevel":
                self?.receiveBatteryLevel(result: result)
            case "getDeviceInfo":
                self?.receiveDeviceInfo(result: result)

            // ── APNs ──
            case "getApnsToken":
                result(self?.apnsToken)

            // ── Notifications ──
            case "getNotifications":
                self?.getDeliveredNotifications(result: result)
            case "removeDeliveredNotification":
                if let args = call.arguments as? [String: Any],
                   let id = args["id"] as? String {
                    self?.removeDeliveredNotification(id: id, result: result)
                } else {
                    result(FlutterError(code: "INVALID_ARGS", message: "Missing 'id'", details: nil))
                }
            case "removeAllDeliveredNotifications":
                self?.removeAllDeliveredNotifications(result: result)

            // ── Phone / Calls ──
            case "getPhoneState":
                self?.getPhoneState(result: result)

            // ── Bluetooth ──
            case "getBluetoothDevices":
                self?.getBluetoothDevices(result: result)

            // ── WiFi ──
            case "getWifiName":
                self?.getWifiName(result: result)

            // ── Notification Listener Settings ──
            case "openNotificationListenerSettings":
                self?.openNotificationListenerSettings(result: result)

            // ── Screen Mirror ──
            case "startScreenMirror":
                self?.startScreenMirror(call: call, result: result)
            case "stopScreenMirror":
                self?.stopScreenMirror(result: result)

            // ── iOS cannot support these Android-specific features ──
            case "getSmsThreads", "getCallLog", "getRunningApps":
                result([])
            case "isNotificationListenerEnabled", "isAccessibilityServiceEnabled", "getSmsPermissions":
                result(false)
            case "sendSms", "injectTouch", "injectKey", "injectScroll":
                result(FlutterMethodNotImplemented)
            default:
                result(FlutterMethodNotImplemented)
            }
        }

        // ── Event Channels ───────────────────────────────────────────────
        // Main events: notifications, calls, screen_mirror
        EventChannel(name: "com.conduit.mobile/events", binaryMessenger: messenger)
            .setStreamHandler(mainStreamHandler)

        // SMS events (Android-only on iOS; handler exists for API symmetry)
        EventChannel(name: "com.conduit.mobile/sms_events", binaryMessenger: messenger)
            .setStreamHandler(smsStreamHandler)

        // ── APNs push notification permissions ───────────────────────────
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) { granted, error in
            if granted {
                DispatchQueue.main.async {
                    application.registerForRemoteNotifications()
                }
            }
        }

        // ── Observe local notifications delivered while app is in foreground ──
        UNUserNotificationCenter.current().delegate = self

        GeneratedPluginRegistrant.register(with: self)
        return super.application(application, didFinishLaunchingWithOptions: launchOptions)
    }

    // MARK: - Battery

    private func receiveBatteryLevel(result: FlutterResult) {
        UIDevice.current.isBatteryMonitoringEnabled = true
        let device = UIDevice.current
        if device.batteryState == .unknown {
            result(-1)
        } else {
            result(Int(device.batteryLevel * 100))
        }
    }

    // MARK: - Device Info

    private func receiveDeviceInfo(result: FlutterResult) {
        let info: [String: Any] = [
            "name": UIDevice.current.name,
            "os": UIDevice.current.systemName,
            "version": UIDevice.current.systemVersion,
            "model": UIDevice.current.model
        ]
        result(info)
    }

    // MARK: - Notifications

    private func getDeliveredNotifications(result: @escaping FlutterResult) {
        UNUserNotificationCenter.current().getDeliveredNotifications { notifications in
            let mapped: [[String: Any]] = notifications.map { notification in
                let content = notification.request.content
                return [
                    "id": notification.request.identifier,
                    "app": content.threadIdentifier.isEmpty
                        ? (Bundle.main.bundleIdentifier ?? "unknown")
                        : content.threadIdentifier,
                    "title": content.title,
                    "body": content.body,
                    "timestamp": Int(notification.date.timeIntervalSince1970)
                ]
            }
            DispatchQueue.main.async {
                result(mapped)
            }
        }
    }

    private func removeDeliveredNotification(id: String, result: @escaping FlutterResult) {
        UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: [id])
        result(true)
    }

    private func removeAllDeliveredNotifications(result: @escaping FlutterResult) {
        UNUserNotificationCenter.current().removeAllDeliveredNotifications()
        result(true)
    }

    // MARK: - Call State (CXCallObserver)

    private func getPhoneState(result: @escaping FlutterResult) {
        result(lastCallState)
    }

    func callObserver(_ callObserver: CXCallObserver, callChanged call: CXCall) {
        var state: String
        if call.hasEnded {
            state = "idle"
        } else if call.hasConnected {
            state = "active"
        } else {
            state = "ringing"
        }
        lastCallState = [
            "state": state,
            "number": call.handle.value
        ]

        // Push call state through the EventChannel so Flutter CallService receives it
        let event: [String: Any] = [
            "type": "call_state",
            "state": state,
            "number": call.handle.value
        ]
        mainStreamHandler.send(type: "calls", data: event)
    }

    // MARK: - Screen Mirror (ReplayKit)

    private func startScreenMirror(call: FlutterMethodCall, result: @escaping FlutterResult) {
        guard !isScreenCapturing else {
            result(false)
            return
        }

        // RPScreenRecorder is only available on iOS 11+; Podfile targets 13.0 so safe.
        let recorder = RPScreenRecorder.shared()

        guard recorder.isAvailable else {
            result(FlutterError(code: "NOT_AVAILABLE",
                                message: "Screen recording is not available on this device",
                                details: nil))
            return
        }

        recorder.startCapture(handler: { [weak self] (sampleBuffer, type, error) in
            guard let self = self, error == nil else { return }

            if type == .screen {
                // Extract the pixel buffer and send metadata through the EventChannel.
                // Sending full frames as raw bytes over EventChannel is expensive;
                // we send metadata + a base64 JPEG snapshot at the configured interval.
                // For production, a more efficient path (e.g. adding the sample buffer
                // directly to a VideoToolBox encoder) would be preferable.
                guard let imageBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
                let ciImage = CIImage(cvImageBuffer: imageBuffer)
                let context = CIContext(options: [.useSoftwareRenderer: false])
                guard let cgImage = context.createCGImage(ciImage, from: ciImage.extent) else { return }

                let uiImage = UIImage(cgImage: cgImage)
                // Resize to reduce bandwidth
                let maxWidth: CGFloat = 640
                let scale = min(maxWidth / uiImage.size.width, 1.0)
                let newSize = CGSize(width: uiImage.size.width * scale,
                                     height: uiImage.size.height * scale)
                let renderer = UIGraphicsImageRenderer(size: newSize)
                let resized = renderer.image { _ in
                    uiImage.draw(in: CGRect(origin: .zero, size: newSize))
                }

                if let jpegData = resized.jpegData(compressionQuality: 0.5) {
                    let base64 = jpegData.base64EncodedString()
                    let event: [String: Any] = [
                        "type": "screen_mirror_frame",
                        "frame": base64,
                        "width": Int(newSize.width),
                        "height": Int(newSize.height),
                        "timestamp": Int(Date().timeIntervalSince1970 * 1000)
                    ]
                    self.mainStreamHandler.send(type: "screen_mirror", data: event)
                }
            }
        }, completionHandler: { [weak self] error in
            DispatchQueue.main.async {
                if let error = error {
                    result(FlutterError(code: "CAPTURE_FAILED",
                                        message: error.localizedDescription,
                                        details: nil))
                } else {
                    self?.isScreenCapturing = true
                    result(true)
                }
            }
        })
    }

    private func stopScreenMirror(result: @escaping FlutterResult) {
        guard isScreenCapturing else {
            result(true)
            return
        }

        RPScreenRecorder.shared().stopCapture { [weak self] error in
            DispatchQueue.main.async {
                self?.isScreenCapturing = false
                self?.screenCaptureTimer?.invalidate()
                self?.screenCaptureTimer = nil
                if let error = error {
                    result(FlutterError(code: "STOP_FAILED",
                                        message: error.localizedDescription,
                                        details: nil))
                } else {
                    result(true)
                }
            }
        }
    }

    // MARK: - Bluetooth (CoreBluetooth)

    private func getBluetoothDevices(result: @escaping FlutterResult) {
        guard centralManager.state == .poweredOn else {
            result([])
            return
        }
        let services: [CBUUID] = [
            CBUUID(string: "1800"), // Generic Access
            CBUUID(string: "1801"), // Generic Attribute
            CBUUID(string: "180A"), // Device Information
            CBUUID(string: "180F"), // Battery Service
            CBUUID(string: "1812"), // Human Interface Device
            CBUUID(string: "FEF5"), // Apple AllSources
        ]
        let peripherals = centralManager.retrieveConnectedPeripherals(withServices: services)
        let devices: [[String: Any]] = peripherals.map { peripheral in
            [
                "name": peripheral.name ?? "Unknown",
                "address": peripheral.identifier.uuidString,
                "type": "ble"
            ]
        }
        result(devices)
    }

    func centralManagerDidUpdateState(_ central: CBCentralManager) {
        // State updated; next getBluetoothDevices call will use current state
    }

    // MARK: - WiFi (NEHotspotNetwork)

    private func getWifiName(result: @escaping FlutterResult) {
        NEHotspotNetwork.fetchCurrent { network in
            DispatchQueue.main.async {
                result(network?.ssid ?? "Unknown")
            }
        }
    }

    // MARK: - Notification Listener Settings (iOS limitation)

    private func openNotificationListenerSettings(result: @escaping FlutterResult) {
        DispatchQueue.main.async {
            let alert = UIAlertController(
                title: "Not Available on iOS",
                message: "iOS does not provide a NotificationListenerService equivalent. Notifications are delivered directly to the app via APNs push notifications. This setting is only available on Android.",
                preferredStyle: .alert
            )
            alert.addAction(UIAlertAction(title: "OK", style: .default))
            self.window?.rootViewController?.present(alert, animated: true)
        }
        result(true)
    }

    // MARK: - APNs Token

    override func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        let token = deviceToken.map { String(format: "%02.2hhx", $0) }.joined()
        self.apnsToken = token
        print("APNs Token: \(token)")
        super.application(application, didRegisterForRemoteNotificationsWithDeviceToken: deviceToken)
    }

    override func application(_ application: UIApplication, didReceiveRemoteNotification userInfo: [AnyHashable : Any], fetchCompletionHandler completionHandler: @escaping (UIBackgroundFetchResult) -> Void) {
        print("Received remote notification: \(userInfo)")
        completionHandler(.newData)
    }
}

// MARK: - UNUserNotificationCenterDelegate

/// Captures notifications delivered while the app is in the foreground and
/// forwards them through the EventChannel so Flutter's NotificationService
/// receives them in real time.
extension AppDelegate: UNUserNotificationCenterDelegate {

    func userNotificationCenter(_ center: UNUserNotificationCenter,
                                willPresent notification: UNNotification,
                                withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        let content = notification.request.content
        let event: [String: Any] = [
            "type": "notification",
            "action": "post",
            "id": notification.request.identifier,
            "device_id": "phone",
            "app": content.threadIdentifier.isEmpty
                ? (Bundle.main.bundleIdentifier ?? "unknown")
                : content.threadIdentifier,
            "title": content.title,
            "body": content.body,
            "timestamp": Int(notification.date.timeIntervalSince1970)
        ]
        mainStreamHandler.send(type: "notifications", data: event)

        // Also show the notification banner as normal
        completionHandler([.banner, .sound])
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter,
                                didReceive response: UNNotificationResponse,
                                withCompletionHandler completionHandler: @escaping () -> Void) {
        // User tapped on a notification — optionally forward a dismiss/open event
        let event: [String: Any] = [
            "type": "notification",
            "action": "open",
            "id": response.notification.request.identifier
        ]
        mainStreamHandler.send(type: "notifications", data: event)
        completionHandler()
    }
}
