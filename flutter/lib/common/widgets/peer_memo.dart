import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hbb/common.dart';
import 'package:flutter_hbb/common/formatter/id_formatter.dart';
import 'package:flutter_hbb/models/peer_model.dart';
import 'package:flutter_hbb/models/peer_tab_model.dart';
import 'package:flutter_hbb/models/platform_model.dart';
import 'package:get/get.dart';
import 'package:provider/provider.dart';

const _kMemoKey = 'rd-memo';

/// Bumped on every save so the cards showing a memo read it again.
final _memoVersion = 0.obs;

String peerMemo(String id) {
  _memoVersion.value;
  return bind.mainGetPeerOptionSync(id: id, key: _kMemoKey).trim();
}

/// The memo button beside the power button on a peer card; green when the PC has a memo.
Widget buildMemoButton(BuildContext context, Peer peer) {
  final PeerTabModel peerTabModel = Provider.of(context);
  if (peerTabModel.multiSelectionMode) return const SizedBox.shrink();
  return Obx(() {
    final memo = peerMemo(peer.id);
    final color = Theme.of(context).textTheme.titleLarge?.color;
    return Tooltip(
      message: memo.isEmpty ? '메모 쓰기' : memo,
      waitDuration: const Duration(milliseconds: 500),
      child: InkWell(
        borderRadius: BorderRadius.circular(14),
        onTap: () => editPeerMemo(peer.id),
        child: SizedBox(
          width: 28,
          height: 28,
          child: Icon(
              memo.isEmpty ? Icons.sticky_note_2_outlined : Icons.sticky_note_2,
              size: 17,
              color: memo.isEmpty ? color?.withOpacity(0.45) : MyTheme.accent),
        ),
      ),
    );
  });
}

/// One line of a peer's memo, or nothing when it has none.
class PeerMemoLine extends StatelessWidget {
  final String id;
  final TextStyle? style;
  final TextAlign textAlign;

  const PeerMemoLine(this.id,
      {Key? key, this.style, this.textAlign = TextAlign.start})
      : super(key: key);

  @override
  Widget build(BuildContext context) {
    return Obx(() {
      final memo = peerMemo(id);
      if (memo.isEmpty) return const SizedBox.shrink();
      return Tooltip(
        message: memo,
        waitDuration: const Duration(milliseconds: 500),
        child: Text(memo.replaceAll('\n', ' '),
            style: style,
            textAlign: textAlign,
            maxLines: 1,
            overflow: TextOverflow.ellipsis),
      );
    });
  }
}

/// The PC's ID with a button that copies it.
class PeerIdCopy extends StatelessWidget {
  final String id;
  final TextStyle? style;
  final MainAxisAlignment alignment;

  const PeerIdCopy(this.id,
      {Key? key, this.style, this.alignment = MainAxisAlignment.start})
      : super(key: key);

  @override
  Widget build(BuildContext context) {
    // Sized to its content: it sits in rows that give it unbounded width.
    return Row(
      mainAxisSize: MainAxisSize.min,
      mainAxisAlignment: alignment,
      children: [
        Text(formatID(id), style: style, maxLines: 1),
        Tooltip(
          message: '고유번호 복사',
          waitDuration: const Duration(milliseconds: 500),
          child: InkWell(
            borderRadius: BorderRadius.circular(10),
            onTap: () {
              Clipboard.setData(ClipboardData(text: id));
              showToast('고유번호를 복사했습니다: ${formatID(id)}');
            },
            child: Padding(
              padding: const EdgeInsets.all(3),
              child: Icon(Icons.copy_rounded,
                  size: (style?.fontSize ?? 12) + 1, color: style?.color),
            ),
          ),
        ),
      ],
    );
  }
}

void editPeerMemo(String id) {
  final controller =
      TextEditingController(text: bind.mainGetPeerOptionSync(id: id, key: _kMemoKey));
  gFFI.dialogManager.show((setState, close, context) {
    Future<void> submit() async {
      await bind.mainSetPeerOption(
          id: id, key: _kMemoKey, value: controller.text.trim());
      _memoVersion.value++;
      close();
    }

    return CustomAlertDialog(
      title: Text('메모 · ${formatID(id)}'),
      content: SizedBox(
        width: 420,
        child: TextField(
          controller: controller,
          autofocus: true,
          minLines: 4,
          maxLines: 8,
          keyboardType: TextInputType.multiline,
          decoration: const InputDecoration(
            hintText: '이 컴퓨터에 대한 설명을 적어 두세요. (예: 사무실 2층 회계 PC)',
            border: OutlineInputBorder(),
          ),
        ),
      ),
      actions: [
        dialogButton('Cancel', onPressed: close, isOutline: true),
        dialogButton('저장', onPressed: submit),
      ],
      onCancel: close,
    );
  });
}
