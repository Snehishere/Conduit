import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:conduit/widgets/status_badge.dart';
import 'package:conduit/theme/app_theme.dart';

void main() {
  Widget buildBadge(BadgeStatus status) {
    return MaterialApp(
      theme: AppTheme.dark(),
      home: Scaffold(
        body: StatusBadge(status: status),
      ),
    );
  }

  testWidgets('renders connected status', (tester) async {
    await tester.pumpWidget(buildBadge(BadgeStatus.connected));
    expect(find.text('Connected'), findsOneWidget);
  });

  testWidgets('renders syncing status', (tester) async {
    await tester.pumpWidget(buildBadge(BadgeStatus.syncing));
    expect(find.text('Syncing'), findsOneWidget);
  });

  testWidgets('renders offline status', (tester) async {
    await tester.pumpWidget(buildBadge(BadgeStatus.offline));
    expect(find.text('Offline'), findsOneWidget);
  });

  testWidgets('renders error status', (tester) async {
    await tester.pumpWidget(buildBadge(BadgeStatus.error));
    expect(find.text('Error'), findsOneWidget);
  });
}
