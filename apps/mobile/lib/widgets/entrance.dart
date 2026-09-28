import 'package:flutter/material.dart';

/// A one-shot fade / slide / scale entrance animation.
///
/// This exists instead of `flutter_animate`'s `.animate()` because
/// `_AnimateState._restart` (flutter_animate 4.5.2) unconditionally does
/// `Future.delayed(widget.delay, () => _play())` from `initState`, even when
/// the delay is zero. That leaves a `Timer` pending in `FakeAsync` whenever a
/// widget test tears the tree down before the next event-loop turn, which trips
/// the binding invariant
/// `A Timer is still pending even after the widget tree was disposed.` and
/// fails the test for reasons that have nothing to do with the widget under
/// test.
///
/// Driving an [AnimationController] directly gives the same visual result and
/// only ever schedules work through the scheduler, which the test binding
/// drains deterministically via `pump()`. The controller is disposed with the
/// widget, so nothing survives teardown.
class Entrance extends StatefulWidget {
  const Entrance({
    super.key,
    required this.child,
    this.delay = Duration.zero,
    this.duration = const Duration(milliseconds: 400),
    this.slideY = 0.0,
    this.beginScale,
    this.curve = Curves.easeOutQuad,
  });

  final Widget child;

  /// Time to wait before the animation starts.
  final Duration delay;

  /// Length of the animation itself, excluding [delay].
  final Duration duration;

  /// Starting vertical offset, expressed as a fraction of the child's height.
  /// Animates to 0. Zero disables the slide.
  final double slideY;

  /// Starting scale, animating to 1.0. Null disables the scale.
  final double? beginScale;

  final Curve curve;

  @override
  State<Entrance> createState() => _EntranceState();
}

class _EntranceState extends State<Entrance>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller;
  late final Animation<double> _progress;

  @override
  void initState() {
    super.initState();
    final total = widget.delay + widget.duration;
    final delayFraction = total.inMicroseconds <= 0
        ? 0.0
        : (widget.delay.inMicroseconds / total.inMicroseconds).clamp(0.0, 1.0);

    _controller = AnimationController(vsync: this, duration: total);
    _progress = CurvedAnimation(
      parent: _controller,
      curve: Interval(delayFraction, 1.0, curve: widget.curve),
    );
    _controller.forward();
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: _progress,
      child: widget.child,
      builder: (context, child) {
        final t = _progress.value.clamp(0.0, 1.0);
        final beginScale = widget.beginScale;

        Widget result = Opacity(opacity: t, child: child);
        if (beginScale != null) {
          result = Transform.scale(
            scale: beginScale + (1.0 - beginScale) * t,
            child: result,
          );
        }
        if (widget.slideY != 0) {
          // FractionalTranslation (rather than Align + FractionalTranslation)
          // so the widget never tries to size itself to an unbounded height
          // inside a scroll view.
          result = FractionalTranslation(
            translation: Offset(0, widget.slideY * (1.0 - t)),
            child: result,
          );
        }
        return result;
      },
    );
  }
}
