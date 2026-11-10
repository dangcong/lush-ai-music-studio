# Lush AI Music Studio — Web

Frontend của Lush AI Music Studio: React 19 + TypeScript (strict) + Vite. Giao tiếp với Control Plane qua REST API mô tả trong [`docs/API_CONTRACTS.md`](../../docs/API_CONTRACTS.md).

## Yêu cầu

- Node.js 20+ (đang phát triển trên Node 24), npm.
- Backend (`apps/api`) chạy ở `http://127.0.0.1:8080` khi phát triển.

## Chạy dev

```bash
npm install
npm run dev        # http://localhost:5173
```

Vite proxy mọi request `/api` tới backend, nên cookie phiên `lush_session` luôn cùng origin với trang web.

## Build

```bash
npm run build      # tsc --noEmit + vite build -> dist/
npm run preview    # xem thử bản build
```

Khi triển khai, phục vụ thư mục `dist/` dưới dạng SPA (mọi đường dẫn không phải file trả về `index.html`) và reverse proxy `/api` tới backend trên cùng origin.

## Test và lint

```bash
npm test                                   # vitest (jsdom), chạy một lần
npm run test:watch                         # chế độ watch
npx vitest run src/api/client.test.ts      # một file
npm run lint                               # eslint
```

Test mock `fetch`, không cần backend.

## Biến môi trường

| Biến | Mặc định | Ý nghĩa |
|---|---|---|
| `VITE_API_PROXY` | `http://127.0.0.1:8080` | Địa chỉ backend mà dev server proxy `/api` tới. Chỉ dùng cho `npm run dev`. |

Đặt trong file `.env` (xem `.env.example`) hoặc trực tiếp khi chạy lệnh.

## Cấu trúc

```
src/
  api/          client.ts (fetch + lỗi {error:{code,message}} + xử lý 401), types.ts (khớp API_CONTRACTS.md),
                api.ts (endpoint), hooks.ts (React Query + polling), mutations.ts
  auth/         AuthContext: phiên đăng nhập, 401 ở bất kỳ đâu -> về /login
  player/       PlayerContext (một thẻ <audio> duy nhất cho cả app) + PlayerBar cố định ở đáy
  features/create/  form Tạo nhạc: formState.ts (logic thuần), GenerateForm, RecentJobs
  components/   Layout, Sidebar, Modal, JobCard, TrackCard, Waveform, ui.tsx…
  pages/        Login, Create, Library, TrackDetail, Queue, ComingSoon, admin/*
  styles/       CSS thuần + CSS variables (tokens.css)
  test/         setup.ts, utils.tsx (mock fetch, dữ liệu mẫu)
```

## Quy ước

- Genre, preset, model, LoRA luôn lấy từ API — không hardcode danh sách trong web.
- Audio chỉ được truy cập qua `/api/v1/artifacts/{id}/content`.
- Job chưa xong được poll mỗi 2 giây; khi không còn job đang chờ/chạy thì dừng poll.
- Giao diện tiếng Việt, giữ nguyên thuật ngữ chuyên môn tiếng Anh; code và tên biến tiếng Anh.
