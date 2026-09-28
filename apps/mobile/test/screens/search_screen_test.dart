import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:conduit/screens/search_screen.dart';
import 'package:conduit/services/file_service.dart';
import 'package:conduit/services/notification_service.dart';
import 'package:conduit/services/clipboard_service.dart';
import 'package:conduit/theme/app_theme.dart';

Widget _app(Widget home) {
  return MultiProvider(
    providers: [
      ChangeNotifierProvider<FileService>(create: (_) => FileService()),
      ChangeNotifierProvider<NotificationService>(
        create: (_) => NotificationService(),
      ),
      ChangeNotifierProvider<ClipboardService>(
        create: (_) => ClipboardService(),
      ),
    ],
    child: MaterialApp(
      theme: AppTheme.dark(),
      home: home,
    ),
  );
}

void main() {
  testWidgets('shows prompt before typing', (tester) async {
    await tester.pumpWidget(_app(const SearchScreen()));
    await tester.pump();

    expect(find.byType(TextField), findsOneWidget);
    expect(find.text('Search files, notifications, clipboard'), findsOneWidget);
    expect(
      find.text('Search files, notifications, and clipboard history'),
      findsOneWidget,
    );
  });

  testWidgets('shows no-results message for unknown query', (tester) async {
    await tester.pumpWidget(_app(const SearchScreen()));
    await tester.pump();

    await tester.enterText(find.byType(TextField), 'zzz-no-match');
    await tester.pump();

    expect(find.text('No results for "zzz-no-match"'), findsOneWidget);
  });
}
