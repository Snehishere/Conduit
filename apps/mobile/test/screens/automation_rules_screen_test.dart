import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:conduit/screens/automation_rules_screen.dart';
import 'package:conduit/services/automation_service.dart';
import 'package:conduit/models/automation_rule.dart';
import 'package:conduit/theme/app_theme.dart';

Widget _app(AutomationService svc) {
  return ChangeNotifierProvider<AutomationService>.value(
    value: svc,
    child: MaterialApp(
      theme: AppTheme.dark(),
      home: const AutomationRulesScreen(),
    ),
  );
}

AutomationRule _rule({
  String id = 'rule_1',
  String name = 'Low battery alert',
  bool enabled = true,
}) {
  return AutomationRule(
    id: id,
    name: name,
    trigger: const Trigger(type: TriggerType.batteryLevel, below: 20),
    action: const AutomationAction(
      type: ActionType.sendNotification,
      title: 'Low battery',
      body: 'Phone below 20%',
    ),
    enabled: enabled,
  );
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  setUp(() {
    SharedPreferences.setMockInitialValues({});
  });

  testWidgets('shows empty state when there are no rules', (tester) async {
    await tester.pumpWidget(_app(AutomationService()));
    await tester.pump();

    expect(find.text('Automation'), findsOneWidget);
    expect(find.text('Trigger and action rules'), findsOneWidget);
    expect(find.text('No automation rules'), findsOneWidget);
    expect(find.text('Create rule'), findsOneWidget);
    expect(find.byType(FloatingActionButton), findsOneWidget);
  });

  testWidgets('renders a seeded rule and toggles it off', (tester) async {
    final svc = AutomationService();
    svc.addRule(_rule());

    await tester.pumpWidget(_app(svc));
    await tester.pump();

    expect(find.text('Low battery alert'), findsOneWidget);
    expect(find.text('rule_1'), findsOneWidget);
    expect(find.text('Battery Level'), findsOneWidget);
    expect(find.text('Send Notification'), findsOneWidget);
    expect(find.text('Rules'), findsOneWidget);
    expect(find.text('Active'), findsOneWidget);
    expect(find.text('Paused'), findsOneWidget);

    final switchFinder = find.byType(Switch);
    expect(tester.widget<Switch>(switchFinder).value, isTrue);

    await tester.tap(switchFinder);
    await tester.pump();

    expect(tester.widget<Switch>(switchFinder).value, isFalse);
    expect(svc.rules.first.enabled, isFalse);
  });

  testWidgets('FAB opens the create-rule bottom sheet', (tester) async {
    await tester.pumpWidget(_app(AutomationService()));
    await tester.pump();

    await tester.tap(find.byType(FloatingActionButton));
    await tester.pumpAndSettle();

    expect(find.text('New automation rule'), findsOneWidget);
    expect(find.text('When this happens...'), findsOneWidget);
    expect(find.text('Select the event that fires this rule'), findsOneWidget);
    expect(find.text('Device Connected'), findsOneWidget);
    expect(find.text('Next'), findsOneWidget);
  });
}
