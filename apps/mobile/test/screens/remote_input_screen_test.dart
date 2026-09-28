import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:conduit/screens/remote_input_screen.dart';
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
  testWidgets('renders status and inactive trackpad state', (tester) async {
    await tester.pumpWidget(
      _app(
        const RemoteInputScreen(
          deviceId: 'desktop-1',
          deviceName: 'Desktop',
        ),
      ),
    );
    await tester.pump();

    expect(find.text('Remote input: Desktop'), findsOneWidget);
    expect(find.text('Status'), findsOneWidget);
    // 'Disconnected' appears in both the status card and the badge.
    expect(find.text('Disconnected'), findsNWidgets(2));
    expect(find.text('Start'), findsOneWidget);
    expect(find.text('Remote input not active'), findsOneWidget);
expect(
        find.text('Tap Start to control Desktop '
            'with this phone as a trackpad'),
        findsOneWidget,
      );
  });
}
