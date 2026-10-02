import 'dart:convert';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:conduit/services/sms_service.dart';

const _native = MethodChannel('com.conduit.mobile/native');

/// The record shape `MainActivity.getSmsThreads` actually puts on the
/// channel (`MainActivity.kt:369-377`): a flat JSON array whose keys are
/// `address`, `body`, `timestamp` (already seconds), `is_outgoing` (bool) and
/// `read` (bool). Deliberately fake data.
String _androidPayload({bool withNativeId = false}) {
  final sent = <String, Object>{
    'address': '+15550001111',
    'body': 'test body out',
    'timestamp': 1700000000,
    'is_outgoing': true,
    'read': true,
  };
  final received = <String, Object>{
    'address': '+15550001111',
    'body': 'test body in',
    'timestamp': 1699999000,
    'is_outgoing': false,
    'read': false,
  };
  if (withNativeId) {
    sent['id'] = 'sms_41';
    received['id'] = 'sms_40';
  }
  // Android queries `content://sms` ordered by `date DESC`.
  return jsonEncode([sent, received]);
}

void _replyWith(String payload) {
  TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
      .setMockMethodCallHandler(_native, (call) async {
    expect(call.method, 'getSmsThreads');
    return payload;
  });
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_native, null);
  });

  test('getSmsThreads payload keeps id, timestamp, direction and read flag',
      () async {
    _replyWith(_androidPayload());
    final svc = SmsService();

    await svc.loadThreads();

    expect(svc.threads, hasLength(1), reason: 'one thread per address');
    final thread = svc.threads.single;
    expect(thread.address, '+15550001111');
    expect(thread.messages, hasLength(2));

    // Ascending by timestamp: the older message first.
    expect(thread.messages[0].body, 'test body in');
    expect(thread.messages[1].body, 'test body out');

    // The two directions must not collapse into one.
    expect(thread.messages[0].isOutgoing, isFalse);
    expect(thread.messages[1].isOutgoing, isTrue);

    // Real send time, not "now", and in seconds like the rest of the protocol.
    expect(thread.messages[0].timestamp, 1699999000);
    expect(thread.messages[1].timestamp, 1700000000);
    expect(thread.timestamp, 1700000000);

    expect(thread.messages[0].read, isFalse);
    expect(thread.messages[1].read, isTrue);
    expect(thread.unreadCount, 1);
  });

  test('a platform-supplied id is used verbatim', () async {
    _replyWith(_androidPayload(withNativeId: true));
    final svc = SmsService();

    await svc.loadThreads();

    expect(svc.threads.single.messages.map((m) => m.id), ['sms_40', 'sms_41']);
  });

  test('message ids are stable across reloads', () async {
    // A synthetic id that embeds "now" makes every reload look like a new set
    // of messages, and the SQLite primary key (thread_id, msg_id) then
    // accumulates a duplicate row per reload.
    _replyWith(_androidPayload());
    final first = SmsService();
    await first.loadThreads();

    final second = SmsService();
    await second.loadThreads();

    expect(
      second.threads.single.messages.map((m) => m.id),
      first.threads.single.messages.map((m) => m.id),
    );
  });

  test('a millisecond timestamp is normalised to seconds', () async {
    _replyWith(jsonEncode([
      {
        'address': '+15550001111',
        'body': 'test body',
        'timestamp': 1700000000000,
        'is_outgoing': false,
        'read': false,
      },
    ]));
    final svc = SmsService();

    await svc.loadThreads();

    expect(svc.threads.single.messages.single.timestamp, 1700000000);
  });

  test('a raw content://sms style record is understood', () async {
    // The unprefixed cursor column names, in milliseconds, with `type` as the
    // Telephony constant. Kept working so a platform that returns the raw
    // cursor row is not silently mangled.
    _replyWith(jsonEncode([
      {
        '_id': 40,
        'address': '+15550001111',
        'body': 'test body',
        'date': 1699999000000,
        'type': 2,
        'read': 1,
      },
    ]));
    final svc = SmsService();

    await svc.loadThreads();

    final msg = svc.threads.single.messages.single;
    expect(msg.id, '40');
    expect(msg.timestamp, 1699999000);
    expect(msg.isOutgoing, isTrue);
    expect(msg.read, isTrue);
  });

  test('an empty platform reply yields no threads and no error', () async {
    _replyWith('[]');
    final svc = SmsService();
    final errors = <String>[];
    svc.onError(errors.add);

    await svc.loadThreads();

    expect(svc.threads, isEmpty);
    expect(errors, isEmpty);
  });

  group("a peer's sms/mark_read", () {
    Future<SmsService> loaded() async {
      _replyWith(_androidPayload());
      final svc = SmsService();
      await svc.loadThreads();
      return svc;
    }

    test('clears the thread unread count and message flags', () async {
      final svc = await loaded();
      expect(svc.threads.single.unreadCount, 1);
      var notified = 0;
      svc.addListener(() => notified++);

      svc.markThreadRead('+15550001111');

      expect(svc.threads.single.unreadCount, 0);
      expect(svc.threads.single.messages.every((m) => m.read), isTrue);
      expect(notified, 1);
    });

    test('is accepted for a thread the phone keys by address', () async {
      // A peer that synced this thread only ever saw the address.
      final svc = SmsService();
      svc.handleSync([
        {
          'thread_id': 't_local_1',
          'address': '+15550001111',
          'name': null,
          'snippet': 'test body',
          'unread_count': 3,
          'timestamp': 1700000000,
          'messages': [
            {
              'id': 'sms_1',
              'address': '+15550001111',
              'body': 'test body',
              'timestamp': 1700000000,
              'read': false,
              'is_outgoing': false,
            },
          ],
        },
      ]);
      expect(svc.threads.single.threadId, 't_local_1');

      svc.markThreadRead('+15550001111');

      expect(svc.threads.single.unreadCount, 0);
      expect(svc.threads.single.messages.every((m) => m.read), isTrue);
    });

    test('leaves other threads alone', () async {
      _replyWith(jsonEncode([
        {
          'id': 'sms_40',
          'address': '+15550001111',
          'body': 'test body',
          'timestamp': 1700000000,
          'is_outgoing': false,
          'read': false,
        },
        {
          'id': 'sms_90',
          'address': '+15550002222',
          'body': 'other',
          'timestamp': 1700001000,
          'is_outgoing': false,
          'read': false,
        },
      ]));
      final svc = SmsService();
      await svc.loadThreads();

      svc.markThreadRead('+15550001111');

      expect(svc.threads.map((t) => t.unreadCount), [1, 0]);
    });

    test('an unknown thread is ignored', () async {
      final svc = await loaded();
      var notified = 0;
      svc.addListener(() => notified++);

      svc.markThreadRead('+15559998888');

      expect(svc.threads.single.unreadCount, 1);
      expect(notified, 0);
    });

    test('marking an already read thread is a no-op', () async {
      final svc = await loaded();
      svc.markThreadRead('+15550001111');
      var notified = 0;
      svc.addListener(() => notified++);

      svc.markThreadRead('+15550001111');

      expect(notified, 0);
    });
  });
}