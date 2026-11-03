import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_hbb/common.dart';
import 'package:flutter_hbb/common/widgets/dialog.dart';
import 'package:flutter_hbb/models/platform_model.dart';
import 'package:get/get.dart';

Map<String, dynamic> _json(String s) {
  try {
    return jsonDecode(s) as Map<String, dynamic>;
  } catch (_) {
    return {};
  }
}

/// The "원격으로 켜기" row in the general settings: whether other PCs can power this one on
/// through its ipTIME router, with the switch that opens the setup.
class RouterSelfSetting extends StatefulWidget {
  const RouterSelfSetting({Key? key}) : super(key: key);

  @override
  State<RouterSelfSetting> createState() => _RouterSelfSettingState();
}

class _RouterSelfSettingState extends State<RouterSelfSetting> {
  @override
  Widget build(BuildContext context) {
    final status = _json(bind.mainRouterSelfStatus());
    final configured = status['configured'] == true;
    final host = (status['host'] as String?) ?? '';
    return ListTile(
      contentPadding: const EdgeInsets.symmetric(horizontal: 16),
      title: const Text('꺼진 이 컴퓨터를 다른 PC에서 켤 수 있게 하기'),
      subtitle: Text(
        configured
            ? '사용 중 · 공유기 $host'
            : 'ipTIME 공유기에 랜선으로 연결된 컴퓨터에서 쓸 수 있습니다.',
        style: TextStyle(
            fontSize: 12,
            color: configured ? MyTheme.accent : Theme.of(context).hintColor),
      ),
      trailing: Switch(
        value: configured,
        onChanged: (on) async {
          if (on) {
            await showRouterSelfSetup();
          } else {
            await bind.mainRouterSelfOff();
          }
          if (mounted) setState(() {});
        },
      ),
    );
  }
}

/// Finds this PC's router, asks only for its admin ID and password, and sets everything up.
Future<void> showRouterSelfSetup() async {
  final user = TextEditingController();
  final pass = TextEditingController();
  // Empty: the default gateway. Filled with what was searched, so a retry can edit it.
  final router = TextEditingController();
  Map<String, dynamic>? found;
  Map<String, dynamic>? result;
  String? error;
  bool busy = false;
  bool searching = false;

  await gFFI.dialogManager.show((setState, close, context) {
    void search() {
      searching = true;
      found = null;
      error = null;
      bind.mainRouterSelfDetect(router: router.text.trim()).then((s) {
        setState(() {
          searching = false;
          found = _json(s);
          final addr = found?['gateway'] as String?;
          if (router.text.trim().isEmpty && addr != null) router.text = addr;
        });
      });
    }

    if (found == null && !searching) search();

    Future<void> submit() async {
      if (busy || user.text.trim().isEmpty || pass.text.isEmpty) return;
      setState(() {
        busy = true;
        error = null;
      });
      final r = _json(await bind.mainRouterSelfSetup(
          user: user.text.trim(), pass: pass.text, router: router.text.trim()));
      if (r['ok'] == true) {
        // The Windows half needs administrator rights, so it gets its own UAC prompt.
        await bind.mainRouterSelfPrepareWindows();
      }
      setState(() {
        busy = false;
        if (r['ok'] == true) {
          result = r;
        } else {
          error = (r['error'] as String?) ?? '설정하지 못했습니다.';
        }
      });
    }

    Widget row(IconData icon, Color color, String text) => Padding(
          padding: const EdgeInsets.symmetric(vertical: 3),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Icon(icon, size: 16, color: color),
              const SizedBox(width: 8),
              Expanded(child: Text(text, style: const TextStyle(fontSize: 13))),
            ],
          ),
        );
    final ok = MyTheme.accent;
    const bad = Color(0xFFE5484D);
    const warn = Color(0xFFE59A2B);

    List<Widget> body;
    List<Widget> actions;
    if (result != null) {
      final r = result!;
      final warnings = (r['warnings'] as List?)?.cast<String>() ?? [];
      body = [
        row(Icons.check_circle, ok, '공유기 로그인 확인'),
        row(Icons.check_circle, ok,
            r['opened'] == true ? '공유기 원격 관리 포트를 켰습니다' : '공유기 원격 관리 포트가 켜져 있습니다'),
        row(Icons.check_circle, ok, '외부 접속 주소: ${r['host']}'),
        if (r['ddns'] != true)
          row(Icons.info_outline, warn,
              'DDNS가 없어 현재 인터넷 주소를 사용합니다. 주소가 바뀌어도 이 컴퓨터에 한 번 접속하면 자동으로 갱신됩니다.'),
        row(Icons.check_circle, ok, '윈도우 깨우기 설정 (관리자 권한 창에서 "예"를 눌러 주세요)'),
        for (final w in warnings) row(Icons.warning_amber_rounded, warn, w),
        const SizedBox(height: 10),
        Text(
          '다른 PC에서 이 컴퓨터에 한 번 접속하면, 그 다음부터는 꺼져 있을 때 전원 버튼만 누르면 켜집니다.\n\n'
          '켜지지 않으면 이 컴퓨터의 BIOS 설정에서 "Wake on LAN"(또는 "Power On By PCI-E")을 켜 주세요.',
          style: TextStyle(fontSize: 12, color: Theme.of(context).hintColor),
        ),
      ];
      actions = [dialogButton('확인', onPressed: close)];
    } else {
      final f = found;
      body = [
        if (f == null)
          const Row(children: [
            SizedBox.square(
                dimension: 14,
                child: CircularProgressIndicator(strokeWidth: 2)),
            SizedBox(width: 10),
            Text('공유기를 찾는 중…'),
          ])
        else if (f['error'] != null)
          row(Icons.error_outline, bad, f['error'] as String)
        else ...[
          row(
              f['iptime'] == true ? Icons.check_circle : Icons.error_outline,
              f['iptime'] == true ? ok : bad,
              f['iptime'] == true
                  ? '공유기: ${f['model']} (${f['gateway']})'
                  : '공유기(${f['gateway']})가 ipTIME이 아니거나 응답하지 않습니다.'),
          row(
              f['wired'] == true ? Icons.check_circle : Icons.error_outline,
              f['wired'] == true ? ok : bad,
              f['wired'] == true
                  ? '랜선 연결 · MAC ${f['mac']}'
                  : '와이파이 연결입니다. 꺼진 상태에서 켜려면 랜선으로 연결해야 합니다.'),
        ],
        if (f != null && f['iptime'] != true) ...[
          const SizedBox(height: 10),
          const Text(
              '공유기 주소가 다르면 직접 입력하고 다시 찾아 주세요. '
              '공유기 관리 화면에 들어갈 때 쓰는 주소입니다.',
              style: TextStyle(fontSize: 12)),
          Row(
            crossAxisAlignment: CrossAxisAlignment.end,
            children: [
              Expanded(
                child: DialogTextField(
                    title: '공유기 주소',
                    hintText: '예: 192.168.0.1',
                    controller: router),
              ),
              const SizedBox(width: 8),
              dialogButton('다시 찾기',
                      onPressed: () => setState(search), isOutline: true)
                  .marginOnly(bottom: 4),
            ],
          ),
        ],
        const SizedBox(height: 14),
        const Text('공유기 관리자 아이디와 비밀번호만 입력하면 나머지는 자동으로 설정합니다.',
            style: TextStyle(fontSize: 12)),
        const SizedBox(height: 6),
        DialogTextField(title: '공유기 관리자 아이디', controller: user),
        DialogTextField(
            title: '공유기 관리자 비밀번호', obscureText: true, controller: pass),
        if (error != null) row(Icons.error_outline, bad, error!),
        if (busy)
          const LinearProgressIndicator(minHeight: 2).marginOnly(top: 8),
      ];
      final ready = f != null &&
          f['error'] == null &&
          f['iptime'] == true &&
          f['wired'] == true;
      actions = [
        dialogButton('Cancel', onPressed: close, isOutline: true),
        dialogButton('설정하기', onPressed: ready && !busy ? submit : null),
      ];
    }

    return CustomAlertDialog(
      title: const Text('원격으로 켜기 설정'),
      content: SizedBox(
        width: 440,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: body,
        ),
      ),
      actions: actions,
      onSubmit: result != null ? close : submit,
      onCancel: close,
    );
  });
}
