import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_hbb/common.dart';
import 'package:flutter_hbb/models/platform_model.dart';
import 'package:http/http.dart' as http;

/// Kept in the repository, so the list also covers versions newer than this build.
const _kChangelogUrl =
    'https://raw.githubusercontent.com/akxm110/remotedesk/remotedesk/res/remotedesk_changelog.json';

Future<List<Map<String, dynamic>>> _fetchChangelog() async {
  final resp = await http
      .get(Uri.parse(_kChangelogUrl))
      .timeout(const Duration(seconds: 10));
  if (resp.statusCode != 200) throw 'HTTP ${resp.statusCode}';
  return (jsonDecode(utf8.decode(resp.bodyBytes)) as List)
      .cast<Map<String, dynamic>>();
}

/// Every release so far, newest first, in a scrolling dialog.
void showChangelog() async {
  final current = await bind.mainGetVersion();
  var future = _fetchChangelog();
  gFFI.dialogManager.show((setState, close, context) {
    final scroll = ScrollController();
    return CustomAlertDialog(
      title: const Text('업데이트 내역'),
      contentBoxConstraints: const BoxConstraints(maxWidth: 560),
      content: SizedBox(
        width: 520,
        height: 440,
        child: FutureBuilder<List<Map<String, dynamic>>>(
          future: future,
          builder: (context, snapshot) {
            if (snapshot.hasError) {
              return Column(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  const Text('업데이트 내역을 불러오지 못했습니다. 인터넷 연결을 확인하세요.'),
                  const SizedBox(height: 10),
                  dialogButton('다시 시도',
                      onPressed: () =>
                          setState(() => future = _fetchChangelog()),
                      isOutline: true),
                ],
              );
            }
            if (!snapshot.hasData) {
              return const Center(child: CircularProgressIndicator());
            }
            return Scrollbar(
              controller: scroll,
              thumbVisibility: true,
              child: ListView(
                controller: scroll,
                padding: const EdgeInsets.only(right: 14),
                children: [
                  for (final release in snapshot.data!)
                    _ReleaseSection(release, current),
                ],
              ),
            );
          },
        ),
      ),
      actions: [dialogButton('닫기', onPressed: close)],
      onCancel: close,
      onSubmit: close,
    );
  });
}

class _ReleaseSection extends StatelessWidget {
  final Map<String, dynamic> release;
  final String current;

  const _ReleaseSection(this.release, this.current);

  @override
  Widget build(BuildContext context) {
    final version = '${release['version'] ?? ''}';
    final items = (release['items'] as List?)?.cast<String>() ?? [];
    final cmp = _compare(version, current);
    final hint = Theme.of(context).hintColor;
    Widget? badge;
    if (cmp > 0) {
      badge = _badge('새 버전', Colors.white, MyTheme.accent);
    } else if (cmp == 0) {
      badge = _badge('사용 중', MyTheme.accent, MyTheme.accent.withOpacity(0.12));
    }
    return Container(
      margin: const EdgeInsets.only(bottom: 12),
      padding: const EdgeInsets.fromLTRB(14, 12, 14, 10),
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(8),
        border: Border.all(
            color: cmp == 0
                ? MyTheme.accent.withOpacity(0.6)
                : Theme.of(context).dividerColor),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(children: [
            Text('v$version',
                style:
                    const TextStyle(fontWeight: FontWeight.bold, fontSize: 15)),
            if (badge != null) ...[const SizedBox(width: 8), badge],
            const Spacer(),
            Text('${release['date'] ?? ''}',
                style: TextStyle(fontSize: 12, color: hint)),
          ]),
          const SizedBox(height: 6),
          for (final item in items)
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 2),
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text('•  ', style: TextStyle(color: MyTheme.accent)),
                  Expanded(
                      child: Text(item,
                          style: const TextStyle(fontSize: 13, height: 1.4))),
                ],
              ),
            ),
        ],
      ),
    );
  }

  Widget _badge(String text, Color fg, Color bg) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 2),
        decoration:
            BoxDecoration(color: bg, borderRadius: BorderRadius.circular(10)),
        child: Text(text,
            style: TextStyle(
                color: fg, fontSize: 11, fontWeight: FontWeight.bold)),
      );

  /// Compares the first version in [a] (a range like "1.5.1 ~ 1.5.8" counts as its start) with
  /// [b]: positive when [a] is newer.
  static int _compare(String a, String b) {
    if (a.contains('~')) return -1;
    List<int> parts(String v) =>
        v.split('.').map((p) => int.tryParse(p.trim()) ?? 0).toList();
    final x = parts(a), y = parts(b);
    for (var i = 0; i < 3; i++) {
      final d = (i < x.length ? x[i] : 0) - (i < y.length ? y[i] : 0);
      if (d != 0) return d;
    }
    return 0;
  }
}
