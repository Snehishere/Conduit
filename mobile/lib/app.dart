import 'package:flutter/material.dart';
import 'screens/home_screen.dart';

class ConduitApp extends StatelessWidget {
  const ConduitApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Conduit',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        brightness: Brightness.dark,
        primarySwatch: Colors.teal,
        scaffoldBackgroundColor: const Color(0xFF08080C),
        cardColor: const Color(0xFF101018),
        colorScheme: const ColorScheme.dark(
          primary: Color(0xFF2DD4BF),
          secondary: Color(0xFF5EEAD4),
          surface: Color(0xFF101018),
        ),
        fontFamily: 'Inter',
        appBarTheme: const AppBarTheme(
          backgroundColor: Color(0xFF0E0E14),
          elevation: 0,
        ),
      ),
      home: const HomeScreen(),
    );
  }
}
