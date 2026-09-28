import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:conduit/screens/screen_mirror_screen.dart';
import 'package:conduit/services/websocket_service.dart';
import 'package:conduit/theme/app_theme.dart';

Widget _app(Widget home) {
  return ChangeNotifierProvider<WebSocketService>(
    create: (_) => WebSocketService(),
    child: MaterialApp(
      theme: AppTheme.dark(),
      home: home,
    ),
  );
}

void main() {
  testWidgets('renders mirror controls and inactive state', (tester) async {
    await tester.pumpWidget(
      _app(
        const ScreenMirrorScreen(
          deviceId: 'desktop-1',
          deviceName: 'Desktop',
        ),
      ),
    );
    await tester.pump();

    expect(find.text('Screen mirror: Desktop'), findsOneWidget);
    expect(find.text('Quality'), findsOneWidget);
    expect(find.text('FPS'), findsOneWidget);
    expect(find.text('Medium (720p)'), findsOneWidget);
    expect(find.text('15 FPS'), findsOneWidget);
    expect(find.text('Start'), findsOneWidget);
    expect(find.text('Screen mirroring is not active'), findsOneWidget);
    expect(
      find.text('Tap Start to show the Desktop screen here'),
      findsOneWidget,
    );
  });
}
