import 'dart:convert';
import 'dart:io';

import 'package:file_picker/file_picker.dart';
import 'package:flutter_hbb/common.dart';
import 'package:flutter_hbb/models/platform_model.dart';
import 'package:image/image.dart' as img;

/// A photo the user chose for a peer card. It wins over the wallpaper the peer sends, and a
/// connection never replaces it.
const kPeerCardImageKey = 'rd-card-image';
const _kWallpaperKey = 'wallpaper-thumb';
const _kMaxWidth = 400;

/// Asks for an image file and keeps a small copy of it as the card's photo. True when one was set.
Future<bool> pickPeerCardImage(String id) async {
  try {
    final result = await FilePicker.platform
        .pickFiles(type: FileType.image, allowMultiple: false);
    final path = result?.files.single.path;
    if (path == null) return false;
    var image = img.decodeImage(await File(path).readAsBytes());
    if (image == null) {
      showToast('이미지 파일을 읽지 못했습니다');
      return false;
    }
    if (image.width > _kMaxWidth) {
      image = img.copyResize(image, width: _kMaxWidth);
    }
    final jpg = img.encodeJpg(image, quality: 82);
    await bind.mainSetPeerOption(
        id: id, key: kPeerCardImageKey, value: base64Encode(jpg));
    return true;
  } catch (e) {
    showToast('카드 사진을 바꾸지 못했습니다: $e');
    return false;
  }
}

/// Drops the chosen photo and the stored wallpaper; the next connection brings the peer's
/// current wallpaper again.
Future<void> resetPeerCardImage(String id) async {
  await bind.mainSetPeerOption(id: id, key: kPeerCardImageKey, value: '');
  await bind.mainSetPeerOption(id: id, key: _kWallpaperKey, value: '');
}
