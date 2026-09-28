import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:conduit/screens/messages_screen.dart';
import 'package:conduit/services/sms_service.dart';
import 'package:conduit/theme/app_theme.dart';

Widget _app(Widget home) {
  return ChangeNotifierProvider<SmsService>(
    create: (_) => SmsService(),
    child: MaterialApp(
      theme: AppTheme.dark(),
      home: home,
    ),
  );
}

void main() {
  testWidgets('shows SMS permission prompt when permission missing',
      (tester) async {
    await tester.pumpWidget(_app(const MessagesScreen()));
    await tester.pump();

    expect(find.text('Messages'), findsOneWidget);
    expect(find.text('SMS permission required'), findsOneWidget);
    expect(
        find.text('Allow Conduit to read your SMS messages so they '
            'can be shared with your desktop'),
        findsOneWidget,
      );
    expect(find.text('Grant permission'), findsOneWidget);
    expect(find.byIcon(Icons.sms_outlined), findsOneWidget);
  });
}
