import 'package:flutter/material.dart';
import 'package:flutter_hbb/common.dart';
import 'package:flutter_hbb/desktop/widgets/update_progress.dart';
import 'package:flutter_hbb/models/platform_model.dart';
import 'package:url_launcher/url_launcher.dart';

final Future<String> _currentVersion = bind.mainGetVersion();

/// Shown at the top of the left pane while a newer release is out.
class UpdateBanner extends StatelessWidget {
  /// The new installer's Drive download link.
  final String updateUrl;
  final VoidCallback onClose;

  const UpdateBanner(
      {Key? key, required this.updateUrl, required this.onClose})
      : super(key: key);

  void _update() async {
    if (isWindows && bind.mainIsInstalled()) {
      handleUpdate(updateUrl);
    } else {
      await launchUrl(Uri.parse(updateUrl));
    }
  }

  @override
  Widget build(BuildContext context) {
    const white70 = Color(0xB3FFFFFF);
    return Container(
      margin: const EdgeInsets.fromLTRB(12, 16, 12, 0),
      padding: const EdgeInsets.fromLTRB(14, 14, 10, 10),
      decoration: BoxDecoration(
        gradient: const LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
          colors: [MyTheme.accent, Color(0xFF3DB45A)],
        ),
        borderRadius: BorderRadius.circular(12),
        boxShadow: const [
          BoxShadow(
              color: Color(0x33000000), blurRadius: 10, offset: Offset(0, 3))
        ],
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Container(
                width: 34,
                height: 34,
                decoration: const BoxDecoration(
                    color: Color(0x33FFFFFF), shape: BoxShape.circle),
                child: const Icon(Icons.system_update_alt_rounded,
                    color: Colors.white, size: 19),
              ),
              const SizedBox(width: 10),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    const Text('새로운 업데이트가 있습니다',
                        style: TextStyle(
                            color: Colors.white,
                            fontWeight: FontWeight.bold,
                            fontSize: 13.5)),
                    const SizedBox(height: 3),
                    FutureBuilder<String>(
                      future: _currentVersion,
                      builder: (context, snapshot) => Text(
                        '${snapshot.hasData ? 'v${snapshot.data} → ' : ''}v${bind.mainGetNewVersion()}',
                        style: const TextStyle(color: white70, fontSize: 12),
                      ),
                    ),
                  ],
                ),
              ),
              Tooltip(
                message: '나중에',
                child: InkWell(
                  borderRadius: BorderRadius.circular(10),
                  onTap: onClose,
                  child: const Padding(
                    padding: EdgeInsets.all(2),
                    child: Icon(Icons.close_rounded, color: white70, size: 16),
                  ),
                ),
              ),
            ],
          ),
          const SizedBox(height: 12),
          Padding(
            padding: const EdgeInsets.only(right: 4),
            child: SizedBox(
              width: double.infinity,
              height: 34,
              child: ElevatedButton(
                style: ElevatedButton.styleFrom(
                  backgroundColor: Colors.white,
                  foregroundColor: MyTheme.accent,
                  elevation: 0,
                  shape: RoundedRectangleBorder(
                      borderRadius: BorderRadius.circular(8)),
                ),
                onPressed: _update,
                child: const Text('지금 업데이트',
                    style:
                        TextStyle(fontWeight: FontWeight.bold, fontSize: 13)),
              ),
            ),
          ),
          const SizedBox(height: 4),
        ],
      ),
    );
  }
}

/// "업데이트 확인" at the bottom of the left pane, with the running version.
class UpdateCheckButton extends StatefulWidget {
  /// Called when the check found a newer release, so a closed banner comes back.
  final VoidCallback onFound;

  const UpdateCheckButton({Key? key, required this.onFound}) : super(key: key);

  @override
  State<UpdateCheckButton> createState() => _UpdateCheckButtonState();
}

class _UpdateCheckButtonState extends State<UpdateCheckButton> {
  bool _checking = false;

  Future<void> _check() async {
    setState(() => _checking = true);
    final result = await bind.mainCheckUpdateNow();
    if (!mounted) return;
    setState(() => _checking = false);
    if (result.startsWith('error:')) {
      showToast('업데이트를 확인하지 못했습니다. 인터넷 연결을 확인하세요.');
    } else if (result.isEmpty) {
      showToast('최신 버전을 사용 중입니다.');
    } else {
      widget.onFound();
      showToast('새 버전 v$result 이 있습니다.');
    }
  }

  @override
  Widget build(BuildContext context) {
    final color = Theme.of(context).hintColor;
    return TextButton.icon(
      onPressed: _checking ? null : _check,
      style: TextButton.styleFrom(
        foregroundColor: color,
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
        minimumSize: Size.zero,
        shape: const StadiumBorder(),
      ),
      icon: _checking
          ? SizedBox.square(
              dimension: 12,
              child: CircularProgressIndicator(strokeWidth: 1.6, color: color))
          : const Icon(Icons.refresh_rounded, size: 15),
      label: FutureBuilder<String>(
        future: _currentVersion,
        builder: (context, snapshot) => Text(
          _checking
              ? '확인 중…'
              : '업데이트 확인${snapshot.hasData ? ' · v${snapshot.data}' : ''}',
          style: const TextStyle(fontSize: 12),
        ),
      ),
    );
  }
}
