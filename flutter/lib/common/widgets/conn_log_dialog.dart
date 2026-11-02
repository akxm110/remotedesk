import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hbb/common.dart';
import 'package:flutter_hbb/models/platform_model.dart';

/// Every connection to [id] so far: how each one got through, or where and why it failed.
void showConnLog(String id) {
  gFFI.dialogManager.show((setState, close, context) {
    final log = bind.mainGetConnLog(id: id);
    final scroll = ScrollController();
    // Newest entries are at the bottom.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (scroll.hasClients) scroll.jumpTo(scroll.position.maxScrollExtent);
    });
    return CustomAlertDialog(
      title: Text('접속 로그 · $id'),
      contentBoxConstraints: const BoxConstraints(maxWidth: 700),
      content: Container(
        width: 680,
        height: 420,
        decoration: BoxDecoration(
          color: Theme.of(context).colorScheme.background,
          borderRadius: BorderRadius.circular(6),
          border: Border.all(color: Theme.of(context).dividerColor),
        ),
        child: Scrollbar(
          controller: scroll,
          thumbVisibility: true,
          child: SingleChildScrollView(
            controller: scroll,
            padding: const EdgeInsets.all(10),
            child: SizedBox(
              width: double.infinity,
              child: log.isEmpty
                  ? Text('아직 기록이 없습니다. 이 PC에 접속하면 과정이 여기에 남습니다.',
                      style: TextStyle(color: Theme.of(context).hintColor))
                  : SelectableText.rich(TextSpan(
                      children: _logSpans(log),
                      style: const TextStyle(
                          fontFamily: 'Consolas', fontSize: 12.5, height: 1.5),
                    )),
            ),
          ),
        ),
      ),
      actions: [
        dialogButton('지우기', onPressed: () {
          bind.mainClearConnLog(id: id);
          setState(() {});
        }, isOutline: true),
        dialogButton('복사', onPressed: () {
          Clipboard.setData(ClipboardData(text: log));
          showToast(translate('Copied'));
        }, isOutline: true),
        dialogButton('닫기', onPressed: close),
      ],
      onCancel: close,
    );
  });
}

List<TextSpan> _logSpans(String log) {
  return log.split('\n').map((line) {
    TextStyle? style;
    if (line.contains('──── 접속 시도')) {
      style = const TextStyle(fontWeight: FontWeight.bold);
    } else if (line.contains('[error]') ||
        line.contains('실패') ||
        line.trimLeft().startsWith('→')) {
      style = const TextStyle(color: Color(0xFFE5484D));
    } else if (line.contains('연결됨') || line.contains('로그인 성공')) {
      style = const TextStyle(color: Color(0xFF2E9E5B));
    }
    return TextSpan(text: '$line\n', style: style);
  }).toList();
}
