import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_hbb/models/peer_tab_model.dart';
import 'package:get/get.dart';
import 'package:provider/provider.dart';

import '../../common.dart';
import '../../common/formatter/id_formatter.dart';
import '../../models/peer_model.dart';
import '../../models/platform_model.dart';

/// Peers a wake request was sent for, until they come online or [_kWakeTimeout] passes.
final wakingPeers = <String>{}.obs;
final _wakeTimers = <String, Timer>{};
const _kWakeTimeout = Duration(minutes: 3);

/// The power button between the ID and the menu. It is enabled only while the peer is offline.
Widget buildPowerButton(BuildContext context, Peer peer) {
  final PeerTabModel peerTabModel = Provider.of(context);
  if (peerTabModel.multiSelectionMode) return const SizedBox.shrink();
  return Obx(() {
    final waking = wakingPeers.contains(peer.id);
    if (waking && peer.online) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _onWokeUp(peer.id));
    }
    final enabled = !peer.online && !waking;
    final color = Theme.of(context).textTheme.titleLarge?.color;
    return Tooltip(
      message: enabled
          ? '컴퓨터 전원 켜기'
          : (waking ? '컴퓨터 전원이 켜지는 중입니다' : '컴퓨터가 켜져 있습니다'),
      waitDuration: const Duration(milliseconds: 500),
      child: InkWell(
        borderRadius: BorderRadius.circular(14),
        onTap: enabled ? () => _confirmWake(peer) : null,
        child: SizedBox(
          width: 28,
          height: 28,
          child: Icon(Icons.power_settings_new,
              size: 18,
              color: enabled ? MyTheme.accent : color?.withOpacity(0.2)),
        ),
      ),
    ).marginOnly(right: 2);
  });
}

/// Covers the computer picture on a grid card while the peer is booting.
Widget buildWakingOverlay(Peer peer) {
  return Obx(() {
    if (!wakingPeers.contains(peer.id)) return const SizedBox.shrink();
    return Positioned.fill(
      child: Container(
        color: Colors.black.withOpacity(0.6),
        child: const Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            BootDots(size: 40, color: Colors.white),
            SizedBox(height: 10),
            Text('컴퓨터 전원이 켜지는 중입니다.',
                style: TextStyle(color: Colors.white, fontSize: 12),
                textAlign: TextAlign.center),
          ],
        ).paddingSymmetric(horizontal: 6),
      ),
    );
  });
}

/// The same notice for the list and tile layouts, which have no picture to cover.
Widget buildWakingLabel(Peer peer) {
  return Obx(() {
    if (!wakingPeers.contains(peer.id)) return const SizedBox.shrink();
    return const Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        BootDots(size: 14, color: MyTheme.accent),
        SizedBox(width: 6),
        Text('컴퓨터 전원이 켜지는 중입니다.',
            style: TextStyle(color: MyTheme.accent, fontSize: 11)),
      ],
    ).marginOnly(right: 6);
  });
}

void _confirmWake(Peer peer) {
  final name = peer.alias.isNotEmpty
      ? peer.alias
      : (peer.hostname.isNotEmpty ? peer.hostname : formatID(peer.id));
  gFFI.dialogManager.show((setState, close, context) {
    submit() {
      close();
      _wake(peer.id);
    }

    return CustomAlertDialog(
      title: const Text('컴퓨터 전원'),
      content: Text('$name 컴퓨터를 켜시겠습니까?'),
      actions: [
        dialogButton('Cancel', onPressed: close, isOutline: true),
        dialogButton('켜기', onPressed: submit),
      ],
      onSubmit: submit,
      onCancel: close,
    );
  });
}

void _wake(String id) async {
  wakingPeers.add(id);
  final asked = await bind.mainWakePeer(id: id);
  if (asked > 0) {
    _wakeTimers[id]?.cancel();
    _wakeTimers[id] = Timer(_kWakeTimeout, () {
      _wakeTimers.remove(id);
      if (wakingPeers.remove(id)) {
        _showWakeInfo('컴퓨터가 켜지지 않았습니다.\n\n'
            '켜려는 컴퓨터가 랜선(유선)으로 연결되어 있는지, '
            'BIOS에서 Wake on LAN이 켜져 있는지 확인해 주세요.');
      }
    });
    return;
  }
  wakingPeers.remove(id);
  if (asked == -1) {
    _showWakeInfo('이 컴퓨터는 아직 원격으로 켤 수 없습니다.\n\n'
        '컴퓨터가 켜져 있을 때 한 번 접속해 두면, 다음부터 여기서 켤 수 있습니다.');
  } else if (asked == 0) {
    _showWakeInfo('같은 공유기에 켜져 있는 RemoteDesk 컴퓨터가 없어서 켤 수 없습니다.\n\n'
        '꺼진 컴퓨터는 같은 공유기에 연결된 다른 컴퓨터가 대신 켜 줍니다.');
  } else {
    _showWakeInfo('서버에 연결하지 못했습니다. 잠시 후 다시 시도해 주세요.');
  }
}

void _onWokeUp(String id) {
  _wakeTimers.remove(id)?.cancel();
  if (wakingPeers.remove(id)) showToast('컴퓨터가 켜졌습니다.');
}

void _showWakeInfo(String text) {
  gFFI.dialogManager.show((setState, close, context) {
    return CustomAlertDialog(
      title: const Text('컴퓨터 전원'),
      content: Text(text),
      actions: [dialogButton('OK', onPressed: close)],
      onSubmit: close,
      onCancel: close,
    );
  });
}

/// Dots circling like the Windows boot screen.
class BootDots extends StatefulWidget {
  final double size;
  final Color color;

  const BootDots({Key? key, required this.size, required this.color})
      : super(key: key);

  @override
  State<BootDots> createState() => _BootDotsState();
}

class _BootDotsState extends State<BootDots>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller = AnimationController(
      vsync: this, duration: const Duration(milliseconds: 2400))
    ..repeat();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: _controller,
      builder: (context, _) => CustomPaint(
        size: Size.square(widget.size),
        painter: _BootDotsPainter(_controller.value, widget.color),
      ),
    );
  }
}

class _BootDotsPainter extends CustomPainter {
  final double progress;
  final Color color;

  _BootDotsPainter(this.progress, this.color);

  @override
  void paint(Canvas canvas, Size size) {
    final dotRadius = size.width / 14;
    final orbit = size.width / 2 - dotRadius;
    final center = size.center(Offset.zero);
    final paint = Paint()..color = color;
    for (var i = 0; i < 5; i++) {
      // Each dot trails the previous one, and all speed up at the bottom and slow at the top.
      final t = ((progress - i * 0.08) % 1.0 + 1.0) % 1.0;
      final angle =
          math.pi / 2 + 2 * math.pi * Curves.easeInOutSine.transform(t);
      canvas.drawCircle(
          center + Offset(math.cos(angle), math.sin(angle)) * orbit,
          dotRadius,
          paint);
    }
  }

  @override
  bool shouldRepaint(_BootDotsPainter oldDelegate) =>
      oldDelegate.progress != progress || oldDelegate.color != color;
}
