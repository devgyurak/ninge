<!-- Korean translation of docs/PLAN.md (approved). docs/PLAN.md is canonical; change both together. -->

# ninge — 터미널 네이티브 NDS 에뮬레이터

> 프로젝트명 `ninge`(발음: "닌지")는 hinge(경첩)를 비튼 이름이다. 두 화면을 잇는 경첩이라는 접이식 기기의 구조에서 따왔다. crate명·바이너리명·저장소명 모두 `ninge`를 사용한다. README·로고에서 첫 글자 N을 특정 회사 상표처럼 강조하지 않는다.

## 1. 프로젝트 개요

### 1.1 목표
- Rust로 **Nintendo DS 에뮬레이터 코어를 처음부터 직접 구현**한다. 외부 에뮬레이터 코어(melonDS, DeSmuME, libretro 코어 등)는 사용하지 않는다.
- 프론트엔드는 **터미널(TUI)** 이며, 대상 터미널은 **Ghostty와 kitty 두 개로 한정**한다.
- 플랫폼: macOS 1급 지원(Apple Silicon), Linux는 CI 검증. **Windows 지원은 계획되어 있다.**
- GUI 에뮬레이터가 제공하지 못하는 **TUI 네이티브 기능(통합 디버거, 키보드 중심 조작)** 을 차별점으로 삼는다.

### 1.2 비목표 (Non-goals)
- GBA 에뮬레이션 및 DS의 GBA 호환 모드
- DSi 확장 기능
- Wi-Fi / 로컬 무선 통신
- kitty graphics protocol을 지원하지 않는 터미널
- 초기 단계의 JIT (인터프리터로 시작한다)

### 1.3 개발 전략 요약
- GBA를 거치지 않고 **NDS로 바로 간다.**
- 이로 인한 두 가지 핵심 리스크와 대응:
  1. CPU를 시스템 맥락 없이 검증해야 함 → 명령어 단위 JSON 테스트 + 자작 bare-metal 테스트 ROM
  2. 처음부터 CPU 두 개를 디버깅해야 함 → **ARM9 단독 bring-up** 후 ARM7 투입

---

## 2. 법적·라이선스 원칙 (모든 작업에 우선 적용)

| 규칙 | 내용 |
|---|---|
| ROM 미포함 | 상용 ROM, 추출 데이터, 스크린샷을 저장소·이슈·테스트 fixture에 포함하지 않는다. ROM 입수 경로를 안내하거나 링크하지 않는다. |
| BIOS/펌웨어 미포함 | 닌텐도 BIOS·펌웨어 덤프를 포함하지 않는다. **HLE BIOS + direct boot + 합성 펌웨어**로 덤프 없이 동작하는 것을 기본으로 한다. 사용자 덤프는 선택 옵션. |
| 클린룸 지향 | 1차 문서는 GBATEK. melonDS 등 GPL 코드는 **문제 해결 방식 이해용으로만** 읽고, 코드를 옮기거나 라인 단위로 번역하지 않는다. |
| 상표 | 프로젝트명·crate명·바이너리명에 Nintendo, DS, Pokémon 등 상표를 넣지 않는다. README에서는 서술적 언급(호환 대상 설명)만 최소한으로 사용한다. |
| 수익화 금지 | 후원·Sponsors·유료 기능을 붙이지 않는다. |
| 라이선스 | **`Apache-2.0`** (확정, GPL 코드 미사용 전제). |

---

## 3. 기술 스택

### 3.1 언어·툴체인
- Rust stable, edition 2024 (또는 최신 stable edition)
- cargo workspace
- 테스트 ROM 빌드: **BlocksDS** (DS 홈브루 SDK), `arm-none-eabi` 어셈블러

### 3.2 주요 crate (후보)

| 영역 | crate | 용도 |
|---|---|---|
| TUI 레이아웃 | `ratatui` | 패널·레이아웃·위젯 |
| 터미널 IO | `crossterm` | raw mode, kitty keyboard protocol(`KeyboardEnhancementFlags`), 마우스 |
| 게임패드 | `gilrs` | 터미널을 거치지 않고 OS에서 직접 입력 |
| 오디오 | `cpal` | 오디오 출력, 링버퍼 |
| 직렬화 | `serde` + 바이너리 포맷(bincode 등) | 세이브스테이트 |
| 병렬화 | `rayon` | 3D 래스터라이저 스캔라인 병렬 처리 |
| 압축/인코딩 | `flate2`, `base64` | kitty graphics `o=z` 전송 |
| 에러 | `thiserror`(core), `anyhow`(bin) | |
| 테스트 | `insta`(스냅샷), `proptest`(속성 기반), `criterion`(벤치) | |

- kitty graphics protocol은 **직접 구현**한다. `ratatui-image`는 참고용. 60fps 경로는 전용 구현이 필요하다.
- `core` crate는 위 IO 관련 crate에 의존하지 않는다.

### 3.3 터미널 기능 사용 범위 (Ghostty / kitty)

| 기능 | 사용 방식 | 비고 |
|---|---|---|
| Kitty graphics protocol | 같은 image id로 프레임 데이터 교체, Unicode placeholder로 셀 그리드에 배치 | 로컬: shared memory(`t=s`), 그 외: zlib 직접 전송(`t=d`, `o=z`). **Ghostty의 shm 지원 범위 확인 필요** |
| Kitty keyboard protocol | press / repeat / release 이벤트 수신 | 게임 입력의 전제 조건 |
| 마우스 | SGR 1006 + SGR-Pixels(1016) | 하단 화면 터치 좌표 매핑. 지원 여부 런타임 감지 |

---

## 4. 아키텍처

### 4.1 Workspace 구조

```
ninge/
├── crates/
│   ├── core/        # 에뮬레이터 코어. IO 없음. 순수 로직.
│   │   ├── cpu/         # ARMv4T/v5TE 공통 인터프리터 (const generic)
│   │   ├── bus/         # ARM9 버스 뷰, ARM7 버스 뷰, 공유 메모리
│   │   ├── scheduler/   # 이벤트 기반 스케줄러
│   │   ├── gpu2d/       # 엔진 A/B
│   │   ├── gpu3d/       # geometry engine, rasterizer
│   │   ├── spu/         # 사운드
│   │   ├── cart/        # 카트리지 프로토콜, AUXSPI 세이브
│   │   ├── spi/         # 펌웨어(합성), 터치 컨트롤러, 전원
│   │   ├── hle/         # HLE BIOS(SWI), direct boot
│   │   └── debug/       # 디버거 훅 (브레이크포인트, 트레이스)
│   ├── runner/      # headless 실행기. 테스트·트레이스·골든 프레임
│   ├── frontend/    # TUI 앱: 렌더러, 입력, 오디오, 레이아웃, 디버거 패널
│   └── tracediff/   # 트레이스 비교 도구
├── testroms/        # 자작 테스트 ROM 소스 (BlocksDS / asm)
├── tests/           # 통합 테스트, 골든 프레임 해시
└── docs/            # 하드웨어 노트(직접 작성한 스펙 요약)
```

### 4.2 코어 설계 원칙
1. **IO 없는 순수 코어.** 입력: 버튼/터치 상태. 출력: 프레임 버퍼 2개, 오디오 샘플.
2. **단일 소유 구조.** 최상위 `Nds` 구조체가 모든 상태를 소유한다. `Rc<RefCell<_>>` 금지. 접근은 `&mut self`.
3. **CPU는 버스 trait에 대해 제네릭**으로 구현해 monomorphize한다. 동적 디스패치는 hot path에서 금지.
4. **ARMv4T/v5TE 공통 구현.** `Cpu<const ARM9: bool>` 형태. 변형별 디코드 테이블은 `build.rs` 또는 `const fn`으로 컴파일 타임 생성.
5. **모든 상태는 plain data.** serde 직렬화 가능해야 하며, 세이브스테이트는 처음부터 지원한다. 포인터·클로저를 상태에 넣지 않는다.
6. **완전한 결정성.** 같은 ROM + 같은 입력 로그 = 같은 결과. 시스템 시간, 난수, 스레드 타이밍이 코어에 들어가지 않는다. (되감기·리플레이 테스트·run-ahead의 전제)
7. **이벤트 기반 스케줄러.** 타임스탬프 기준 binary heap. ARM9 클럭 = ARM7 클럭 × 2. 사이클 정확도는 목표가 아니며, 타이밍 근사 + 결정성을 목표로 한다.
8. **버스는 공유 상태 위의 두 뷰.** WRAMCNT, VRAM 뱅크 등 소유권이 바뀌는 영역 때문에 ARM9 뷰와 ARM7 뷰를 분리한다.
9. **메모리 fast path.** VRAM 뱅크 매핑 등은 16KB 단위 페이지 테이블로 처리한다.
10. `unsafe`는 성능상 필요가 측정으로 입증된 경우에만, 안전성 근거 주석과 함께 사용한다.

### 4.3 프론트엔드 설계
- **스레드:** emu(코어 실행 + 오디오 페이싱), render(최신 프레임 소비), input. 프레임 전달은 triple buffer. 렌더가 느리면 프레임을 버린다.
- **페이싱:** 오디오 기준. 오디오가 없는 상황에서는 프레임 기준으로 전환.
- **렌더링:** 에뮬레이션(약 60fps)과 터미널 출력 fps를 분리하고, 터미널 쓰기 시간을 측정해 출력 fps를 적응 조절한다.
- **화면 레이아웃 모드:** 세로(원본 배치) / 가로 / 포커스(한쪽 크게, 다른 쪽 작게) / 하단 전용. 단축키로 스왑·전환.
- **입력:**
  - 키보드: kitty keyboard protocol press/release
  - 게임패드: `gilrs`
  - 터치: SGR-Pixels 마우스 → 하단 화면 좌표. 미지원 시 셀 중심 매핑
  - 키보드 가상 커서: hjkl 이동, space 탭, 드래그 모드 토글
  - 마이크(바람 불기): 키 하나에 매핑
- **디버거 패널 (TUI의 핵심 차별점):**
  - ARM9/ARM7 분리: 레지스터, 디스어셈블리, 브레이크포인트, 워치포인트, 스텝 실행
  - 메모리 뷰어·검색
  - VRAM 뱅크 매핑 현황, 타일·팔레트·OAM 뷰
  - IPC FIFO 내용
  - 스케줄러 이벤트 타임라인
  - (Phase 7 이후) 3D 명령 로그, 폴리곤 와이어프레임

---

## 5. 로드맵

각 Phase는 **완료 조건(DoD)** 을 모두 만족해야 다음으로 넘어간다.

### Phase 0. 하네스와 도구
- workspace 골격, CI(빌드·테스트·clippy·fmt)
- 테스트 ROM 툴체인: **libnds 초기화 없이 동작하는 bare-metal 테스트 ROM**을 빌드할 수 있어야 한다. (libnds 초기화는 ARM7·IPC 동기화를 기다리므로 초반에 사용 불가)
- 골든 프레임 테스트 인프라: ROM을 N프레임 실행 → 프레임 버퍼 해시 비교
- TUI 골격: kitty graphics로 임의 프레임 버퍼 출력, 레지스터/디스어셈블리 패널 자리

**DoD:** 더미 프레임 버퍼가 Ghostty·kitty 양쪽에 출력된다. 테스트 ROM 하나가 빌드되고 `runner`가 로드한다.

### Phase 1. CPU
- ARMv4T 공통부(ARM + Thumb): 배럴 시프터, 플래그, 파이프라인(PC+8/+4), 예외, 모드 전환
- ARM7TDMI용 명령어 단위 JSON 테스트 스위트로 검증
- ARMv5TE 확장: BLX, CLZ, QADD 계열, LDRD/STRD, PLD, `LDR PC`/`POP {PC}`의 Thumb 전환(v5만)
- CP15 최소 구현: TCM 설정, 예외 벡터 위치. MPU는 무시
- 디스어셈블러 (디버거 패널용)

**DoD:** JSON 테스트 전부 통과. v5 확장 각각에 대한 자작 테스트 통과.

### Phase 2. 첫 픽셀 (ARM9 단독)
- NDS 롬 헤더 파싱, ARM9 바이너리 direct boot
- 메인 RAM, ITCM/DTCM, VRAM 뱅크 A의 LCDC 매핑
- 엔진 A display mode 2 (VRAM 직접 출력)만 구현
- ARM7은 스텁

**DoD:** 테스트 ROM이 VRAM에 쓴 이미지가 Ghostty에 표시되고, 골든 프레임 테스트로 고정된다.

### Phase 3. 듀얼 CPU와 시스템
- ARM7 투입, 스케줄러로 두 CPU 교대 실행
- 버스 두 뷰, WRAMCNT
- IPC SYNC/FIFO, 인터럽트(IE/IF/IME), 타이머, DMA
- HLE SWI (CpuSet, LZ77 압축 해제, Div 등)
- 엔진 A 텍스트 BG 모드

**DoD:** libnds 기반 홈브루의 콘솔 printf 출력이 정상 표시된다.

### Phase 4. 2D 완성
- 엔진 A/B 전체: 모든 BG 모드, 스프라이트, 확장 팔레트, 윈도우, 블렌딩, master brightness
- VRAM 뱅크 매핑 전체 (페이지 테이블)
- display capture

**DoD:** 2D 기능별 자작 테스트 ROM 골든 프레임 통과.

### Phase 5. 상용 게임 부팅 조건
- 카트리지 프로토콜 (복호화 덤프 기준 롬 읽기 명령)
- AUXSPI 세이브 칩 (EEPROM / FLASH / FRAM) + 타입 감지
- **합성 펌웨어** (사용자 이름, 언어, 터치 보정값)
- RTC, 터치 컨트롤러 SPI, 전원 관리
- 사운드 레지스터 응답만 우선 구현 (사운드 대기로 멈추는 게임 대응)
- 세이브 파일 저장/로드

**DoD:** 포켓몬스터 다이아몬드가 부팅되고, 새 게임을 시작해 저장하며, 재시작 후 그 세이브를 다시 불러온다. 오버월드가 3D이므로 Phase 7 전까지 3D 엔진은 지오메트리 명령을 받기만 하고 그리지 않는 스텁이어도 된다.

### Phase 6. 사운드
- 16채널 믹싱, PCM / ADPCM / PSG, 사운드 캡처
- `cpal` 연결, 오디오 기준 페이싱으로 전환

**DoD:** 테스트 ROM 출력 파형 스냅샷 테스트 통과, 상용 게임 사운드 정상.

### Phase 7. 3D
- Geometry engine: 명령 FIFO, 행렬 스택, 버텍스 변환, 클리핑, 폴리곤 RAM
- Rendering engine: 소프트웨어 래스터라이저. 엣지 규칙, Z/W 버퍼, 텍스처 포맷 7종, 툰 셰이딩, 엣지 마킹, 포그, 안티에일리어싱
- `rayon` 스캔라인 병렬화

**DoD:** 3D 기능별 테스트 ROM 골든 프레임 통과, 3D 상용 게임(포켓몬스터 다이아몬드) 플레이 가능.
> 참고: 4·5세대 포켓몬(DP, HGSS, BW)은 오버월드가 3D이므로 이 Phase가 필수다.

### Phase 8. 성능과 정확도
- 프로파일링 기반 최적화: 메모리 fast path, 인터프리터 hot path
- 필요 시 블록 캐싱. JIT는 macOS `MAP_JIT` 제약을 고려해 마지막 선택지

**DoD:** 목표 기기(Apple Silicon Mac)에서 주요 게임 풀스피드.

### 병행 트랙 (Phase 1부터 계속)
- 디버거 패널 확장
- 트레이스 diff 도구 (레퍼런스 에뮬레이터를 **오라클로** 사용, 최초 분기 지점 자동 탐지)
- 리플레이 기반 회귀 테스트

---

## 6. 리스크와 대응

| 리스크 | 대응 |
|---|---|
| CPU를 시스템 맥락 없이 검증 | JSON 단위 테스트 + 자작 bare-metal 테스트 ROM |
| 두 CPU 동시 디버깅 | ARM9 단독 bring-up 후 ARM7 투입 |
| 첫 화면까지 오래 걸려 동기 저하 | display mode 2로 최단 경로 첫 픽셀 |
| 상용 게임이 이유 없이 멈춤 | 합성 펌웨어, 세이브 칩 감지, 스케줄러 이벤트 로그, 트레이스 diff |
| v5TE 테스트 자료 부족 | 자작 테스트 ROM 우선, 레퍼런스 에뮬레이터 트레이스 비교 |
| GPL 코드 오염 | 클린룸 원칙, GBATEK 기반 자체 스펙 문서(`docs/`) 작성 후 구현 |

---

## 7. 개발 워크플로우 (AI 에이전트용)

### 7.1 하드웨어 기능 구현 표준 절차
1. **스펙 정리:** GBATEK 해당 절을 읽고 `docs/hw/<기능>.md`에 **자기 말로** 요약한다. 레지스터 주소, 비트 필드, 동작, 엣지 케이스, 미확인 사항을 기록한다.
2. **테스트 먼저:** 기능을 검증하는 테스트를 작성한다. CPU는 단위 테스트, 그 외는 `testroms/`에 테스트 ROM + 기대 결과(골든 프레임 해시 또는 메모리 덤프 조건).
3. **구현:** `core`에 구현한다. 원칙(4.2) 위반 여부를 확인한다.
4. **디버거 노출:** 새 상태가 생기면 디버거 패널에서 볼 수 있게 한다.
5. **검증:** 전체 테스트 + 골든 프레임 회귀 + clippy + fmt.
6. **기록:** 설계 결정은 `decision` 라벨의 GitHub 이슈로 남긴다.

### 7.2 버그 조사 표준 절차
1. 입력 리플레이로 재현 고정
2. 트레이스 diff로 레퍼런스와 최초 분기 지점 탐지
3. 분기 지점의 하드웨어 기능을 `docs/hw/`에서 확인, 없으면 스펙 정리부터
4. 재현 테스트를 먼저 추가하고 수정

### 7.3 커밋·PR 규칙
- 한 PR = 한 하드웨어 기능 또는 한 버그
- 코어 변경 PR은 반드시 테스트를 동반한다
- 골든 프레임 해시 변경 시 변경 사유를 PR 설명에 명시한다

---

## 8. 에이전트 룰 (위반 금지)

1. 상용 ROM, BIOS, 펌웨어 덤프, 그 추출물을 저장소에 추가하지 않는다.
2. GPL 에뮬레이터 코드를 복사하거나 라인 단위로 번역하지 않는다. 참고 시 "어떤 문제를 어떻게 풀었는지"만 요약한다.
3. `core` crate에 IO, 시스템 시간, 난수, 스레드 의존성을 추가하지 않는다.
4. 상태에 `Rc`, `RefCell`, 클로저, 원시 포인터를 넣지 않는다. 모든 상태는 serde 직렬화 가능해야 한다.
5. hot path(CPU 실행, 메모리 접근, 픽셀 처리)에 동적 디스패치·힙 할당을 넣지 않는다.
6. `unsafe`는 측정 근거와 `// SAFETY:` 주석 없이 사용하지 않는다.
7. 테스트 없는 코어 변경을 하지 않는다.
8. 스펙이 불확실한 동작은 추측으로 구현하지 않는다. `docs/hw/`에 "미확인"으로 기록하고 `TODO(hw-unverified)` 주석을 남긴다.
9. 현재 Phase의 범위를 넘는 기능을 구현하지 않는다. 필요하면 스텁 + `TODO(phaseN)`.
10. 상표명(Nintendo, DS, Pokémon 등)을 crate명, 바이너리명, 식별자에 쓰지 않는다. 하드웨어 문맥상 필요한 레지스터·부품 명칭은 예외.

---

## 9. 레퍼런스

- **GBATEK** (Martin Korth): GBA/NDS 하드웨어 1차 문서
- **melonDS 블로그**: 하드웨어 기행과 구현 노트 (코드 참조 금지, 개념 참고)
- **NooDS**, **Dust**(Rust): 1인/소규모 NDS 에뮬레이터 구현 사례 (개념 참고)
- **BlocksDS**: 테스트 ROM 빌드용 DS 홈브루 SDK
- ARM Architecture Reference Manual (ARMv5TE), ARM7TDMI / ARM946E-S Technical Reference Manual
- ARM7TDMI 명령어 단위 JSON 테스트 스위트
- kitty 문서: graphics protocol, keyboard protocol
- Ghostty 문서: 지원 프로토콜 범위

---

## 10. 결정 필요 항목

- [x] 프로젝트명: `ninge` (crates.io 사용 가능 확인), 저장소 `github.com/devgyurak/ninge`
- [x] 라이선스: `Apache-2.0`
- [ ] Ghostty의 kitty graphics shared memory 전송 지원 여부 확인 (Phase 0에서 확인, 그 전까지 zlib 직접 전송이 기본)
- [x] 세이브스테이트: 매직 넘버와 포맷 버전을 붙인 바이너리 serde 인코딩. 1.0 전에는 하위 호환 없음, 버전 불일치는 명확한 에러로 거부
- [x] 첫 목표 상용 게임: 포켓몬스터 다이아몬드 (Phase 5 부팅~저장, Phase 7 완전 플레이)
