import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:conduit/widgets/search_bar.dart';
import 'package:conduit/theme/app_theme.dart';

void main() {
  testWidgets('accepts text input', (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.dark(),
        home: Scaffold(
          body: ConduitSearchBar(
            hint: 'Search...',
            onChanged: (_) {},
          ),
        ),
      ),
    );

    await tester.enterText(find.byType(TextField), 'hello');
    await tester.pump();

    expect(find.text('hello'), findsOneWidget);
  });

  testWidgets('shows search icon', (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.dark(),
        home: Scaffold(
          body: ConduitSearchBar(
            hint: 'Search...',
            onChanged: (_) {},
          ),
        ),
      ),
    );

    expect(find.byIcon(Icons.search), findsOneWidget);
  });

  testWidgets('calls onChanged when text changes', (tester) async {
    String changedValue = '';

    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.dark(),
        home: Scaffold(
          body: ConduitSearchBar(
            hint: 'Search...',
            onChanged: (val) => changedValue = val,
          ),
        ),
      ),
    );

    await tester.enterText(find.byType(TextField), 'test');
    await tester.pump();

    expect(changedValue, 'test');
  });
}
