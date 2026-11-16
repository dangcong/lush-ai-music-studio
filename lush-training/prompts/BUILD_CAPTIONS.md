# BUILD_CAPTIONS — Soạn và kiểm tra caption

Dùng sau khi đã có dataset version. Không dùng GPU. Bạn không nghe được audio: mọi mô tả nội dung âm nhạc phải đến từ người dùng, từ metadata có sẵn, hoặc từ một công cụ đã được cấu hình (ghi rõ công cụ và phiên bản).

## Các bước

1. `python scripts/analyze_audio.py --dataset <name> --version <v>` rồi `python scripts/validate_captions.py init --dataset <name> --version <v>`.
2. Đọc phần "Không có" mà `analyze_audio` in ra và nói lại cho người dùng: công cụ nào thiếu, nên trường nào chưa có bằng chứng tự động.
3. Thu thập mô tả từ người dùng theo **nhóm bài** (thư mục, album, phong cách), không hỏi từng segment một: thể loại / subgenre, nhạc cụ chính, mood, nhịp và tempo nếu họ biết, hòa âm, chất âm, độ động, cấu trúc.
4. Với mỗi segment, viết caption từ mô tả đó:
   - tiếng Anh, một câu hoặc danh sách ngắn các cụm từ, cùng bộ từ vựng mà studio dùng khi viết prompt tạo nhạc;
   - chỉ nêu điều có bằng chứng; trường chưa biết để `null` trong metadata và bỏ khỏi caption;
   - không nhắc tới giọng hát, ca sĩ, lời bài hát;
   - mỗi trường metadata có giá trị kèm một mục `evidence` (`human`, `source_metadata` hoặc `tool` + `tool_version`).
   Ghi bằng `validate_captions.py set … --by claude` (không có `--human`).
5. Segment mà mô tả của nhóm có thể không đúng (đoạn intro, đoạn chuyển, số đo khác hẳn các segment cùng track): đặt `--status needs_listening`.
6. `python scripts/validate_captions.py validate --dataset <name> --version <v>` và sửa mọi lỗi.
7. Báo người dùng: số segment theo trạng thái, danh sách cần họ nghe lại kèm lý do cụ thể, các cảnh báo thiếu evidence, các mâu thuẫn đã thấy.
8. Người dùng duyệt. Ghi quyết định của họ bằng `--human --by "<tên họ>" --status approved`. Sửa của họ đi kèm `--human` để vào `human_edits`.
9. Khi mọi segment train/validation đã `approved` và `validate` không lỗi: hỏi người dùng rồi `validate_captions.py lock … --caption-version c1`.

## Không làm

- Không viết "piano", "jazz", "80 bpm" cho một segment chỉ vì đường dẫn file trông như vậy mà không nói rõ nguồn là tên file (`source_metadata`).
- Không đưa tempo ước lượng vào `metadata.tempo_bpm` khi người dùng chưa xác nhận.
- Không tự đặt `approved`.
- Không sửa caption version đã khóa: cần thay đổi thì khóa version mới (`c2`).
- Không viết caption cho tập test.
