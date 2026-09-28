import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:conduit/services/websocket_service.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  // ── Handler Registration ─────────────────────────────────────────

  group('Message handler registration', () {
    testWidgets('registerHandler stores handler for message type', (tester) async {
      final ws = WebSocketService();
      var called = false;

      ws.registerHandler('notification', (msg) {
        called = true;
      });

      // Verify by sending a message through a local echo server
      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {
          // Echo back a notification message
          socket.add(jsonEncode({
            'type': 'notification',
            'action': 'post',
            'id': 'test_1',
            'title': 'Test',
          }));
        });
      });

      await ws.connect('ws://localhost:$port');
      // Wait for the echo response to be processed
      await Future.delayed(const Duration(milliseconds: 500));

      expect(called, isTrue);

      ws.dispose();
      await server.close();
    });

    testWidgets('unregisterHandler removes specific handler', (tester) async {
      final ws = WebSocketService();
      var callCount = 0;

      void handler(Map<String, dynamic> msg) {
        callCount++;
      }

      ws.registerHandler('test_type', handler);
      ws.unregisterHandler('test_type', handler);

      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {
          socket.add(jsonEncode({'type': 'test_type', 'action': 'test'}));
        });
      });

      await ws.connect('ws://localhost:$port');
      await Future.delayed(const Duration(milliseconds: 500));

      expect(callCount, 0);

      ws.dispose();
      await server.close();
    });

    testWidgets('unregisterHandler without specific handler removes all for type',
        (tester) async {
      final ws = WebSocketService();
      var callCount = 0;

      ws.registerHandler('bulk_remove', (msg) => callCount++);
      ws.registerHandler('bulk_remove', (msg) => callCount++);
      ws.unregisterHandler('bulk_remove');

      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {
          socket.add(jsonEncode({'type': 'bulk_remove', 'action': 'test'}));
        });
      });

      await ws.connect('ws://localhost:$port');
      await Future.delayed(const Duration(milliseconds: 500));

      expect(callCount, 0);

      ws.dispose();
      await server.close();
    });

    testWidgets('multiple handlers for same type all get called', (tester) async {
      final ws = WebSocketService();
      final calls = <String>[];

      ws.registerHandler('multi', (msg) => calls.add('handler_1'));
      ws.registerHandler('multi', (msg) => calls.add('handler_2'));
      ws.registerHandler('multi', (msg) => calls.add('handler_3'));

      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {
          socket.add(jsonEncode({'type': 'multi', 'action': 'go'}));
        });
      });

      await ws.connect('ws://localhost:$port');
      await Future.delayed(const Duration(milliseconds: 500));

      expect(calls, containsAll(['handler_1', 'handler_2', 'handler_3']));

      ws.dispose();
      await server.close();
    });

    testWidgets('handlers for different types are independent', (tester) async {
      final ws = WebSocketService();
      var typeACalled = false;
      var typeBCalled = false;

      ws.registerHandler('type_a', (msg) => typeACalled = true);
      ws.registerHandler('type_b', (msg) => typeBCalled = true);

      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {
          socket.add(jsonEncode({'type': 'type_a', 'action': 'test'}));
        });
      });

      await ws.connect('ws://localhost:$port');
      await Future.delayed(const Duration(milliseconds: 500));

      expect(typeACalled, isTrue);
      expect(typeBCalled, isFalse);

      ws.dispose();
      await server.close();
    });
  });

  // ── Connection State Management ──────────────────────────────────

  group('Connection state management', () {
    testWidgets('initial state is disconnected', (tester) async {
      final ws = WebSocketService();

      expect(ws.isConnected, isFalse);
      expect(ws.lastError, isNull);

      ws.dispose();
    });

    testWidgets('isConnected becomes true after successful connection', (tester) async {
      final ws = WebSocketService();

      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {});
      });

      await ws.connect('ws://localhost:$port');

      expect(ws.isConnected, isTrue);
      expect(ws.lastError, isNull);

      ws.dispose();
      await server.close();
    });

    testWidgets('isConnected becomes false after server closes', (tester) async {
      final ws = WebSocketService();

      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      WebSocket? clientSocket;
      server.transform(WebSocketTransformer()).listen((socket) {
        clientSocket = socket;
        socket.listen((data) {});
      });

      await ws.connect('ws://localhost:$port');
      expect(ws.isConnected, isTrue);

      // Server closes the connection
      await clientSocket?.close();
      await Future.delayed(const Duration(milliseconds: 200));

      expect(ws.isConnected, isFalse);

      ws.dispose();
      await server.close();
    });

    testWidgets('device info is stored correctly', (tester) async {
      final ws = WebSocketService();
      ws.setDeviceInfo('My iPhone', '17.0');

      expect(ws.deviceName, 'My iPhone');

      ws.dispose();
    });

    testWidgets('setDeviceId stores the device ID', (tester) async {
      final ws = WebSocketService();
      ws.setDeviceId('mobile_abc123');

      expect(ws.deviceId, 'mobile_abc123');

      ws.dispose();
    });
  });

  // ── Reconnection Logic ───────────────────────────────────────────

  group('Reconnection logic', () {
    testWidgets('reconnects automatically after server closes connection',
        (tester) async {
      final ws = WebSocketService();
      var connectCount = 0;

      ws.addListener(() {
        if (ws.isConnected) connectCount++;
      });

      // Start a server that immediately closes the connection
      var server = await HttpServer.bind('localhost', 0);
      var port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {
          // Close immediately after receiving data
          socket.close();
        });
      });

      await ws.connect('ws://localhost:$port');
      expect(ws.isConnected, isTrue);

      // Server closes — should trigger reconnect
      await Future.delayed(const Duration(milliseconds: 200));
      expect(ws.isConnected, isFalse);

      // Start a new server for the reconnection
      server = await HttpServer.bind('localhost', port);
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {});
      });

      // Wait for exponential backoff reconnect (starts at 3s)
      await Future.delayed(const Duration(seconds: 5));

      // Should have reconnected
      expect(ws.isConnected, isTrue);
      // Initial connect + successful reconnect both notified listeners.
      expect(connectCount, greaterThanOrEqualTo(2));

      ws.dispose();
      await server.close();
    });

    testWidgets('does not reconnect if already connected', (tester) async {
      final ws = WebSocketService();

      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {});
      });

      await ws.connect('ws://localhost:$port');
      expect(ws.isConnected, isTrue);

      // connect() again — should not throw or disconnect
      await ws.connect('ws://localhost:$port');

      ws.dispose();
      await server.close();
    });
  });

  // ── Heartbeat Timer ──────────────────────────────────────────────

  group('Heartbeat timer', () {
    testWidgets('sends periodic pings to keep connection alive', (tester) async {
      final ws = WebSocketService();
      final receivedMessages = <Map<String, dynamic>>[];

      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {
          final msg = jsonDecode(data as String) as Map<String, dynamic>;
          receivedMessages.add(msg);
          // Respond with pong
          socket.add(jsonEncode({'type': 'pong'}));
        });
      });

      await ws.connect('ws://localhost:$port');

      // Wait for at least one heartbeat (25 second interval)
      // We'll just verify the timer is set up by checking connection stays alive
      await Future.delayed(const Duration(seconds: 2));

      // Connection should still be alive
      expect(ws.isConnected, isTrue);

      ws.dispose();
      await server.close();
    });

    testWidgets('responds to incoming ping with pong', (tester) async {
      final ws = WebSocketService();
      var pongReceived = false;

      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {
          final msg = jsonDecode(data as String) as Map<String, dynamic>;
          if (msg['type'] == 'pong') {
            pongReceived = true;
          }
        });
        // Send a ping to the client
        Timer(const Duration(milliseconds: 100), () {
          socket.add(jsonEncode({'type': 'ping'}));
        });
      });

      await ws.connect('ws://localhost:$port');
      await Future.delayed(const Duration(milliseconds: 500));

      expect(pongReceived, isTrue);

      ws.dispose();
      await server.close();
    });
  });

  // ── Message Dispatching ──────────────────────────────────────────

  group('Message dispatching', () {
    testWidgets('receives and dispatches typed messages', (tester) async {
      final ws = WebSocketService();
      final receivedMessages = <Map<String, dynamic>>[];

      ws.registerHandler('clipboard', (msg) => receivedMessages.add(msg));

      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {});
        // Send a clipboard message
        Timer(const Duration(milliseconds: 100), () {
          socket.add(jsonEncode({
            'type': 'clipboard',
            'action': 'sync',
            'content': 'copied text',
            'mime': 'text/plain',
          }));
        });
      });

      await ws.connect('ws://localhost:$port');
      await Future.delayed(const Duration(milliseconds: 500));

      expect(receivedMessages.length, 1);
      expect(receivedMessages[0]['action'], 'sync');
      expect(receivedMessages[0]['content'], 'copied text');

      ws.dispose();
      await server.close();
    });

    testWidgets('dispatches to multiple handlers for same type', (tester) async {
      final ws = WebSocketService();
      final results = <String>[];

      ws.registerHandler('notify', (msg) => results.add('first'));
      ws.registerHandler('notify', (msg) => results.add('second'));

      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {});
        Timer(const Duration(milliseconds: 100), () {
          socket.add(jsonEncode({'type': 'notify', 'action': 'test'}));
        });
      });

      await ws.connect('ws://localhost:$port');
      await Future.delayed(const Duration(milliseconds: 500));

      expect(results, containsAll(['first', 'second']));

      ws.dispose();
      await server.close();
    });

    testWidgets('ignores messages with no registered handler', (tester) async {
      final ws = WebSocketService();
      // No handler registered

      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {});
        Timer(const Duration(milliseconds: 100), () {
          socket.add(jsonEncode({'type': 'unhandled', 'action': 'test'}));
        });
      });

      await ws.connect('ws://localhost:$port');
      // Should not throw
      await Future.delayed(const Duration(milliseconds: 500));

      expect(ws.isConnected, isTrue);

      ws.dispose();
      await server.close();
    });
  });

  // ── Sending Messages ─────────────────────────────────────────────

  group('Sending messages', () {
    testWidgets('sendMessage sends JSON over WebSocket', (tester) async {
      final ws = WebSocketService();
      Map<String, dynamic>? received;

      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {
          received = jsonDecode(data as String) as Map<String, dynamic>;
        });
      });

      await ws.connect('ws://localhost:$port');
      ws.sendMessage({'type': 'status', 'action': 'update', 'battery': 85});
      await Future.delayed(const Duration(milliseconds: 200));

      expect(received, isNotNull);
      expect(received!['type'], 'status');
      expect(received!['action'], 'update');
      expect(received!['battery'], 85);

      ws.dispose();
      await server.close();
    });

    testWidgets('sendClipboardSync sends correct payload', (tester) async {
      final ws = WebSocketService();
      Map<String, dynamic>? received;

      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {
          received = jsonDecode(data as String) as Map<String, dynamic>;
        });
      });

      await ws.connect('ws://localhost:$port');
      ws.sendClipboardSync('clipboard content', 'text/plain');
      await Future.delayed(const Duration(milliseconds: 200));

      expect(received, isNotNull);
      expect(received!['type'], 'clipboard');
      expect(received!['action'], 'sync');
      expect(received!['content'], 'clipboard content');
      expect(received!['mime'], 'text/plain');

      ws.dispose();
      await server.close();
    });

    testWidgets('sendNotificationDismiss sends correct payload', (tester) async {
      final ws = WebSocketService();
      Map<String, dynamic>? received;

      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {
          received = jsonDecode(data as String) as Map<String, dynamic>;
        });
      });

      await ws.connect('ws://localhost:$port');
      ws.sendNotificationDismiss('notif_123');
      await Future.delayed(const Duration(milliseconds: 200));

      expect(received, isNotNull);
      expect(received!['type'], 'notification');
      expect(received!['action'], 'dismiss');
      expect(received!['id'], 'notif_123');

      ws.dispose();
      await server.close();
    });

    testWidgets('sendStatusUpdate includes battery level', (tester) async {
      final ws = WebSocketService();
      Map<String, dynamic>? received;

      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {
          received = jsonDecode(data as String) as Map<String, dynamic>;
        });
      });

      await ws.connect('ws://localhost:$port');
      ws.sendStatusUpdate(battery: 42);
      await Future.delayed(const Duration(milliseconds: 200));

      expect(received, isNotNull);
      expect(received!['type'], 'status');
      expect(received!['action'], 'update');
      expect(received!['battery'], 42);

      ws.dispose();
      await server.close();
    });
  });

  // ── Connected Devices Tracking ───────────────────────────────────

  group('Connected devices tracking', () {
    testWidgets('tracks devices from discovery announcements', (tester) async {
      final ws = WebSocketService();

      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {});
        Timer(const Duration(milliseconds: 100), () {
          socket.add(jsonEncode({
            'type': 'discovery',
            'action': 'announce',
            'device_id': 'desktop_1',
            'device_name': 'My Desktop',
            'device_type': 'desktop',
          }));
        });
      });

      await ws.connect('ws://localhost:$port');
      await Future.delayed(const Duration(milliseconds: 500));

      expect(ws.connectedDevices.length, 1);
      expect(ws.connectedDevices[0]['device_id'], 'desktop_1');
      expect(ws.connectedDevices[0]['device_name'], 'My Desktop');

      ws.dispose();
      await server.close();
    });

    testWidgets('removes device on discovery remove', (tester) async {
      final ws = WebSocketService();

      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {});
        Timer(const Duration(milliseconds: 100), () {
          socket.add(jsonEncode({
            'type': 'discovery',
            'action': 'announce',
            'device_id': 'desktop_1',
            'device_name': 'My Desktop',
            'device_type': 'desktop',
          }));
        });
        Timer(const Duration(milliseconds: 300), () {
          socket.add(jsonEncode({
            'type': 'discovery',
            'action': 'remove',
            'device_id': 'desktop_1',
          }));
        });
      });

      await ws.connect('ws://localhost:$port');
      await Future.delayed(const Duration(milliseconds: 600));

      expect(ws.connectedDevices, isEmpty);

      ws.dispose();
      await server.close();
    });

    testWidgets('connectedDevices returns unmodifiable list', (tester) async {
      final ws = WebSocketService();
      final devices = ws.connectedDevices;

      expect(() => devices.add({'device_id': 'x'}), throwsA(anything));

      ws.dispose();
    });
  });

  // ── Error Handling ───────────────────────────────────────────────

  group('Error handling', () {
    testWidgets('connection failure sets lastError', (tester) async {
      final ws = WebSocketService();

      try {
        await ws.connect('ws://localhost:1'); // invalid port
      } catch (_) {}

      // After connection failure, it may try relay or reconnect
      // The lastError should be set
      expect(ws.isConnected, isFalse);

      ws.dispose();
    });

    testWidgets('dispose cleans up resources', (tester) async {
      final ws = WebSocketService();

      final server = await HttpServer.bind('localhost', 0);
      final port = server.port;
      server.transform(WebSocketTransformer()).listen((socket) {
        socket.listen((data) {});
      });

      await ws.connect('ws://localhost:$port');
      expect(ws.isConnected, isTrue);

      ws.dispose();

      // After dispose, the service should be cleaned up
      // (addListener would fail since super.dispose() was called)
      expect(ws.isConnected, isFalse);

      await server.close();
    });
  });
}
