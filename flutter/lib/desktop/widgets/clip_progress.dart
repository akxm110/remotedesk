import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hbb/common.dart';
import 'package:get/get.dart';

/// "파일을 이곳으로 복사중… 37%" in the remote window's title bar while files copied through the
/// clipboard are moving. Each remote window is its own engine, so one value per window is enough.
class ClipProgress {
  static final text = ''.obs;
  static Timer? _hide;

  static void update(Map<String, dynamic> evt) {
    final pct = int.tryParse('${evt['pct']}') ?? 0;
    final where = evt['dir'] == 'in' ? '이곳으로' : '밖으로';
    text.value = pct >= 100 ? '파일 복사 완료 100%' : '파일을 $where 복사중… $pct%';
    _hide?.cancel();
    // A finished copy fades soon; a stalled or cancelled one once nothing has moved for a while.
    _hide = Timer(Duration(seconds: pct >= 100 ? 3 : 15), () => text.value = '');
  }
}

class ClipProgressLabel extends StatelessWidget {
  const ClipProgressLabel({Key? key}) : super(key: key);

  @override
  Widget build(BuildContext context) {
    return Obx(() {
      final t = ClipProgress.text.value;
      if (t.isEmpty) return const SizedBox.shrink();
      return Padding(
        padding: const EdgeInsets.symmetric(horizontal: 10),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(Icons.file_copy_outlined, size: 14, color: MyTheme.accent),
            const SizedBox(width: 5),
            Text(t,
                style: TextStyle(
                    fontSize: 12,
                    fontWeight: FontWeight.w600,
                    color: MyTheme.accent)),
          ],
        ),
      );
    });
  }
}
