import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import 'app.dart';
import 'theme/theme_provider.dart';
import 'services/websocket_service.dart';
import 'services/notification_service.dart';
import 'services/discovery_service.dart';
import 'services/clipboard_service.dart';
import 'services/encryption_service.dart';
import 'services/file_service.dart';
import 'services/sms_service.dart';
import 'services/call_service.dart';
import 'services/pairing_service.dart';
import 'services/automation_service.dart';
import 'services/audio_stream_service.dart';
import 'services/database_service.dart';
import 'services/native_screen_capture.dart';
import 'models/conduit_notification.dart';

const _nativeChannel = MethodChannel('com.conduit.mobile/native');

Future<void> _broadcastBattery(WebSocketService ws) async {
  try {
    final level = await _nativeChannel.invokeMethod<int>('getBatteryLevel');
    if (level != null) {
      ws.sendStatusUpdate(battery: level);
    }
  } catch (e) {
    debugPrint('Battery broadcast error: $e');
  }
}

void main() async {
  WidgetsFlutterBinding.ensureInitialized();

  // Global Flutter error handler
  FlutterError.onError = (FlutterErrorDetails details) {
    FlutterError.presentError(details);
    debugPrint('[FATAL] Flutter error: ${details.exception}');
    // No crash reporting backend is wired up: errors reach the device log
    // and nowhere else. Settings > Diagnostics says as much.
  };

  runZonedGuarded(() async {
    final encryptionService = EncryptionService();
    await encryptionService.initialize();

    final themeProvider = ThemeProvider();
    await themeProvider.init();

    // Initialize SQLite database first — other services depend on it
    final databaseService = DatabaseService();
    await databaseService.init();

  final websocketService = WebSocketService();
  final notificationService = NotificationService();
  final discoveryService = DiscoveryService();
  final clipboardService = ClipboardService();
  final fileService = FileService();
  final smsService = SmsService();
  final callService = CallService();
  final pairingService = PairingService(encryptionService);
  final automationService = AutomationService();
  final audioStreamService = AudioStreamService();
  await audioStreamService.initialize();

  notificationService.setDatabase(databaseService);
  clipboardService.setDatabase(databaseService);
  fileService.setDatabase(databaseService);
  smsService.setDatabase(databaseService);

  await notificationService.initialize();
  await automationService.loadRules();
  
  try {
    final info = await _nativeChannel.invokeMapMethod<String, dynamic>('getDeviceInfo');
    if (info != null) {
      final name = info['name'] as String? ?? 'Mobile Device';
      final osVersion = info['version'] as String? ?? '1.0.0';
      websocketService.setDeviceInfo(name, osVersion);
    }
  } catch (e) {
    debugPrint('Failed to get device info: $e');
  }

  try {
    final token = await _nativeChannel.invokeMethod<String>('getApnsToken');
    if (token != null) {
      websocketService.setApnsToken(token);
    }
  } catch (e) {
    debugPrint('Failed to get APNs token (likely not iOS or permission denied): $e');
  }

  automationService.attachWebSocket(websocketService);
  websocketService.getSharedSecret = () => pairingService.sharedSecret;
  fileService.setSendFunction(websocketService.sendMessage);
  fileService.setSendBinaryFunction(websocketService.sendBinaryMessage);
  fileService.setGetSharedSecret(() => pairingService.sharedSecret);
  callService.setSendFunction(websocketService.sendMessage);
  smsService.setSendFunction(websocketService.sendMessage);
  audioStreamService.setSendFunction(websocketService.sendMessage);

  // Initialize call service (starts native phone state listener)
  await callService.initialize();

  // Initialize pairing service (loads shared secret from secure storage)
  await pairingService.initialize();

  // Restore the device id the desktop assigned at pairing. Must happen before
  // anything can send: it is the value stamped into `source_device` on every
  // `encrypted` envelope, and the hub resolves the shared secret by it.
  await websocketService.restoreDeviceId();

  // Reconnect to the endpoint the last successful connection used.
  websocketService.autoConnect();

  websocketService.registerHandler('notification', (msg) {
    final action = msg['action'] as String?;
    if (action == 'post') {
      notificationService.addNotification(ConduitNotification(
        id: (msg['id'] as String?) ?? 'notif_${DateTime.now().millisecondsSinceEpoch}',
        deviceId: msg['device_id'] as String? ?? 'unknown',
        app: msg['app'] as String? ?? 'Unknown',
        title: msg['title'] as String? ?? '',
        body: msg['body'] as String? ?? '',
        timestamp: (msg['timestamp'] as num?)?.toInt() ?? DateTime.now().millisecondsSinceEpoch ~/ 1000,
      ));
    } else if (action == 'dismiss') {
      final id = msg['id'] as String?;
      if (id != null) notificationService.dismissNotification(id);
    }
  });

  websocketService.registerHandler('clipboard', (msg) {
    final action = msg['action'] as String?;
    if (action == 'sync') {
      clipboardService.syncFromDevice(
        msg['content'] as String? ?? '',
        msg['mime'] as String? ?? 'text/plain',
        msg['source_device'] as String? ?? 'unknown',
      );
    }
  });

  websocketService.registerHandler('sms', (msg) {
    final action = msg['action'] as String?;
    if (action == 'sync') {
      final threads = msg['threads'] as List<dynamic>?;
      if (threads != null) {
        smsService.handleSync(threads);
      }
    } else if (action == 'new') {
      final threadId = msg['thread_id'] as String?;
      final messageData = msg['message'] as Map<String, dynamic>?;
      if (threadId != null && messageData != null) {
        smsService.handleNewMessage(threadId, messageData);
      }
    } else if (action == 'send') {
      final to = msg['to'] as String?;
      final body = msg['body'] as String?;
      if (to != null && body != null) {
        smsService.sendSms(to, body);
      }
    } else if (action == 'mark_read') {
      // A peer opened the thread; clear its unread state here too, or this
      // device's badge says unread about a thread the user is looking at.
      final threadId = msg['thread_id'] as String?;
      if (threadId != null) {
        smsService.markThreadRead(threadId);
      }
    }
  });

  websocketService.registerHandler('call', (msg) {
    final action = msg['action'] as String?;
    if (action == 'incoming') {
      final callId = msg['call_id'] as String?;
      final number = msg['number'] as String?;
      if (callId == null || number == null) return;
      callService.handleIncomingCall(
        callId,
        number,
        msg['name'] as String?,
      );
    } else if (action == 'answered') {
      final callId = msg['call_id'] as String?;
      if (callId != null) callService.handleCallAnswered(callId);
    } else if (action == 'ended') {
      final callId = msg['call_id'] as String?;
      if (callId != null) callService.handleCallEnded(callId);
    }
  });

  websocketService.registerHandler('audio', (msg) {
    final action = msg['action'] as String?;
    if (action == 'stream_data') {
      final data = msg['data'] as String?;
      if (data != null) {
        audioStreamService.playAudioData(
          data,
          format: msg['format'] as String? ?? 'pcm16',
          sampleRate: (msg['sample_rate'] as num?)?.toInt() ?? 16000,
        );
      }
    } else if (action == 'stream_started') {
      debugPrint('Desktop started audio stream');
    } else if (action == 'stream_stop') {
      audioStreamService.stopPlayback();
    } else if (action == 'route') {
      // Retained for compatibility with older desktop builds. The route hint
      // is logged but not acted on.
      debugPrint('Audio route received: ${msg['route']}');
    }
  });

  websocketService.registerHandler('file', (msg) {
    final action = msg['action'] as String?;
    if (action == 'request') {
      fileService.handleFileRequest(Map<String, dynamic>.from(msg));
    } else if (action == 'chunk') {
      final id = msg['id'] as String?;
      final index = (msg['index'] as num?)?.toInt();
      final data = msg['data'] as String?;
      if (id != null && index != null && data != null) {
        debugPrint('WS: Received legacy base64 chunk, ignored');
      }
    } else if (action == 'chunk_binary') {
      final id = msg['id'] as String?;
      final index = (msg['index'] as num?)?.toInt();
      final data = msg['data'] as List<int>?;
      if (id != null && index != null && data != null) {
        fileService.receiveChunk(id, index, data);
      }
    } else if (action == 'accept') {
      final id = msg['id'] as String?;
      if (id != null) fileService.handleFileAccepted(id);
    } else if (action == 'cancel') {
      final id = msg['id'] as String?;
      if (id != null) fileService.handleFileCancelled(id);
    } else if (action == 'progress') {
      final id = msg['id'] as String?;
      final received = (msg['chunks_received'] as num?)?.toInt();
      if (id != null && received != null) {
        fileService.handleFileProgress(id, received);
      }
    } else if (action == 'resume_ack') {
      final id = msg['id'] as String?;
      final loaded = (msg['chunks_loaded'] as num?)?.toInt();
      if (id != null && loaded != null) {
        fileService.handleResumeAck(id, loaded);
      }
    }
  });

  websocketService.registerHandler('discovery', (msg) {
    final action = msg['action'] as String?;
    if (action == 'announce') {
      discoveryService.deviceFound(DiscoveredDevice(
        id: msg['device_id'] as String? ?? 'unknown',
        name: msg['device_name'] as String? ?? 'Unknown',
        type: msg['device_type'] as String? ?? 'unknown',
        address: msg['address'] as String? ?? '',
        port: (msg['port'] as num?)?.toInt() ?? 9527,
      ));
    }
  });

  // Clipboard auto-sync, automation polling and battery reporting all
  // follow the hub connection state.
  Timer? batteryTimer;
  websocketService.addListener(() {
    if (websocketService.isConnected && !clipboardService.isListening) {
      clipboardService.startListening(
        onChange: (content, mime) {
          websocketService.sendClipboardSync(content, mime);
        },
      );
      automationService.startTriggerDetection();

      // Initialize SMS and sync to desktop
      smsService.initialize().then((_) {
        smsService.syncToDesktop();
      });

      // Broadcast battery level on connect
      _broadcastBattery(websocketService);
      batteryTimer?.cancel();
      batteryTimer = Timer.periodic(
        const Duration(minutes: 5),
        (_) => _broadcastBattery(websocketService),
      );
    } else if (!websocketService.isConnected && clipboardService.isListening) {
      clipboardService.stopListening();
      automationService.stopTriggerDetection();
      batteryTimer?.cancel();
      batteryTimer = null;
    }
  });

  websocketService.registerHandler('automation', (msg) {
    final action = msg['action'] as String?;
    if (action == 'triggered') {
      automationService.handleRuleTriggered(msg);
    }
  });

  websocketService.registerHandler('pairing', (msg) {
    final action = msg['action'] as String?;
    if (action == 'accept') {
      final publicKey = msg['public_key'] as String?;
      if (publicKey != null && publicKey.isNotEmpty) {
        pairingService.completeKeyExchange(publicKey);
      }
    }
  });

  // The desktop can ask this phone to start or stop screen capture.
  websocketService.registerHandler('screen_mirror', (msg) {
    final action = msg['action'] as String?;
    if (action == 'start') {
      _nativeChannel.invokeMethod('startScreenMirror', {
        'quality': msg['quality'] ?? 'medium',
        'fps': msg['fps'] ?? 15,
      });
    } else if (action == 'stop') {
      _nativeChannel.invokeMethod('stopScreenMirror');
    } else if (action == 'touch' || action == 'key' || action == 'scroll') {
      // Input the desktop sends back for this phone to apply. Android
      // applies it through the accessibility service; iOS has no handler
      // for these methods.
      _nativeChannel.invokeMethod(action!, msg);
    }
  });

  // Subscribe to native capture frames for the whole app lifetime. Both natives
  // push frames on `com.conduit.mobile/screen_mirror_events`, which this stream
  // now owns outright — an `EventChannel` delivers every event to a single sink
  // per channel *name*, so the `com.conduit.mobile/events` channel this used to
  // share with notifications and calls had each of its three subscribers
  // silently cancel the other two. Nothing read this one at all, so the phone
  // started capturing and every frame the native side produced was discarded:
  // the desktop's canvas never fills. Registering here rather than inside the
  // mirror screen is deliberate - the relay must outlive any one screen,
  // because the desktop may ask for a stream while the user is elsewhere in the
  // app. The handler is idempotent, so a second attach is harmless.
  NativeScreenCapture.attachTo(websocketService);

  // The desktop sends touch, key and scroll events for this phone to apply.
  // Android only — the iOS handlers return not-implemented.
  websocketService.registerHandler('remote_input', (msg) {
    final action = msg['action'] as String?;
    if (action == 'touch') {
      final x = (msg['x'] as num?)?.toDouble() ?? 0;
      final y = (msg['y'] as num?)?.toDouble() ?? 0;
      final type = msg['type'] as String? ?? 'tap';
      _nativeChannel.invokeMethod('injectTouch', {'x': x, 'y': y, 'actionType': type});
    } else if (action == 'key') {
      final key = msg['key'] as String? ?? '';
      final modifiers = (msg['modifiers'] as List?)?.cast<String>() ?? [];
      _nativeChannel.invokeMethod('injectKey', {'key': key, 'modifiers': modifiers});
    } else if (action == 'scroll') {
      final delta = (msg['delta'] as num?)?.toDouble() ?? 0;
      _nativeChannel.invokeMethod('injectScroll', {'delta': delta});
    }
  });

  runApp(
    MultiProvider(
      providers: [
        Provider.value(value: databaseService),
        ChangeNotifierProvider.value(value: themeProvider),
        ChangeNotifierProvider.value(value: websocketService),
        ChangeNotifierProvider.value(value: notificationService),
        ChangeNotifierProvider.value(value: discoveryService),
        ChangeNotifierProvider.value(value: clipboardService),
        ChangeNotifierProvider.value(value: fileService),
        ChangeNotifierProvider.value(value: smsService),
        ChangeNotifierProvider.value(value: callService),
        ChangeNotifierProvider.value(value: pairingService),
        ChangeNotifierProvider.value(value: automationService),
        ChangeNotifierProvider.value(value: audioStreamService),
        Provider.value(value: encryptionService),
      ],
      child: ConduitApp(themeProvider: themeProvider),
    ),
  );
  }, (error, stack) {
    debugPrint('[FATAL] Unhandled error in zone: $error');
    debugPrint('[FATAL] Stack trace: $stack');
  });
}
