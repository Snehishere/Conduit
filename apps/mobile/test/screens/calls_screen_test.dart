import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:conduit/screens/calls_screen.dart';
import 'package:conduit/services/call_service.dart';
import 'package:conduit/services/audio_stream_service.dart';
import 'package:conduit/theme/app_theme.dart';

Widget _app(CallService callService, AudioStreamService audioStream) {
  return MultiProvider(
    providers: [
      ChangeNotifierProvider<CallService>.value(value: callService),
      ChangeNotifierProvider<AudioStreamService>.value(value: audioStream),
    ],
    child: MaterialApp(
      theme: AppTheme.dark(),
      home: const CallsScreen(),
    ),
  );
}

void main() {
  testWidgets('shows idle state with audio route and streaming controls',
      (tester) async {
    final callService = CallService();
    await tester.pumpWidget(_app(callService, AudioStreamService()));
    await tester.pump();

    expect(find.text('Calls'), findsOneWidget);
    expect(find.text('No active calls'), findsOneWidget);
    expect(find.text('Incoming calls will appear here'), findsOneWidget);
    expect(find.text('AUDIO OUTPUT'), findsOneWidget);
    expect(find.text('AUDIO STREAMING'), findsOneWidget);
    expect(find.text('Phone'), findsOneWidget);
    expect(find.text('Desktop'), findsOneWidget);
    expect(find.text('Bluetooth'), findsOneWidget);
    expect(find.text('Start stream'), findsOneWidget);
    expect(find.text('Playback'), findsOneWidget);

    // Switching the audio route must not require a live connection.
    await tester.tap(find.text('Desktop'));
    await tester.pump();
    expect(callService.currentRoute, AudioOutputRoute.desktop);
  });

  testWidgets('shows incoming call UI and answers the call',
      (tester) async {
    final callService = CallService();
    callService.handleIncomingCall('c1', '+1234567890', 'Alice');

    await tester.pumpWidget(_app(callService, AudioStreamService()));
    await tester.pump();

    expect(find.text('Alice'), findsOneWidget);
    expect(find.text('+1234567890'), findsOneWidget);
    expect(find.text('Incoming call...'), findsOneWidget);
    expect(find.text('Reject'), findsOneWidget);
    expect(find.text('Answer'), findsOneWidget);
    expect(find.text('Forward to Desktop'), findsOneWidget);

    await tester.tap(find.text('Answer'));
    await tester.pump();

    expect(find.text('Call in progress'), findsOneWidget);
    expect(find.text('End'), findsOneWidget);
    expect(find.text('Incoming call...'), findsNothing);
  });

  testWidgets('rejecting an incoming call returns to idle state',
      (tester) async {
    final callService = CallService();
    callService.handleIncomingCall('c2', '+1555123456', 'Bob');

    await tester.pumpWidget(_app(callService, AudioStreamService()));
    await tester.pump();

    expect(find.text('Incoming call...'), findsOneWidget);

    await tester.tap(find.text('Reject'));
    await tester.pump();

    expect(find.text('No active calls'), findsOneWidget);
    expect(find.text('Incoming call...'), findsNothing);
    expect(callService.state, CallState.idle);
  });
}
