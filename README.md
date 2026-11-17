# QuickSwitch

Riot Client hesapları arasında şifre yazmadan geçiş. Her hesabın "Oturumu açık tut" oturumunu saklar, seçtiğin hesabınkini yerine koyar ve oyunu başlatır.

## İndir ve kur

1. Deponun [Releases](../../releases/latest) sayfasını aç.
2. `QuickSwitch_..._x64-setup.exe` dosyasını indir ve çalıştır.
3. Windows "bilinmeyen yayıncı" uyarısı gösterirse **Ek bilgi**, sonra **Yine de çalıştır**. Kurulum dosyası imzasız olduğu için bu uyarı çıkar.
4. Uygulama açılınca sihirbaz ilk hesabı eklemeyi anlatır.

Windows 10 ve 11'de çalışır. Kaldırmak için Windows'un **Uygulamalar** ayarını kullan.

## Kaynak koddan derleme

1. [Rust](https://rustup.rs) ve "Desktop development with C++" bileşeniyle Visual Studio Build Tools kur.
2. `cargo install tauri-cli --version "^2" --locked`
3. Denemek için `cargo tauri dev`, kurulum dosyası için `cargo tauri build` (çıktı: `target\release\bundle\nsis\`).

Testler: `cargo test --workspace`

Yeni sürüm yayınlamak: sürüm numarasını `src-tauri/tauri.conf.json` içinde artırıp `main` dalına gönder. GitHub kurulum dosyasını kendisi derleyip Releases'e ekler. Otomatik güncellemeyi açmak için: [docs/UPDATES.md](docs/UPDATES.md).

## Hesap ekleme

İlk açılışta sihirbaz kendiliğinden gelir; sonra **Hesap ekle** ile açılır. İki yol var:

- **Giriş yaparak ekle (önerilen):** Riot Client giriş ekranında açılır. "Oturumu açık tut" kutusunu işaretleyip giriş yaparsın, uygulama girişi algılar, ad sorar ve oturumu saklar. Şifre saklanmaz.
- **Şifreyle ekle:** Kullanıcı adı ve şifre şifreli saklanır, her geçişte giriş ekranına otomatik yazılır.

Oturumu kaydederken Riot Client kapatılır; oturum diske ancak düzgün kapanışta yazıldığı için bu gerekli.

## Hesap değiştirme

- Pencere: hesaba tıkla ya da adını yazıp Enter'a bas.
- Tepsi simgesi: sağ tık, hesabı seç.
- `Ctrl+Shift+Space`: pencereyi her yerden açar (Ayarlar'dan değiştirilebilir).
- Hesap kısayolu: hesabın ayarlarında (⋯) kısayol kutusuna tıklayıp tuşlara bas, örneğin `Ctrl+Shift+1`. Türkçe klavyede Ctrl+Alt, AltGr ile aynıdır; karakter yazan bir kombinasyon (AltGr+3 = #) kısayol yapılamaz.
- Masaüstü kısayolu: hesabın ayarlarında **Masaüstü kısayolu oluştur**.
- Komut satırı: `quickswitch.exe --switch "Hesap Adı"`.

Listeyi düzenlemek: bir hesabı tutup yukarı ya da aşağı sürükle (klavyede Alt+ok tuşları). Hesapların hem League hem VALORANT'ı kapsıyorsa listenin üstünde **Tümü / League / VALORANT** filtresi çıkar; VALORANT filtresindeyken iki oyunlu bir hesaba tıklamak onu VALORANT ile açar.

Maç açıkken geçiş yapılmaz; **Maç bitince geç** ile sıraya alınabilir.

## Hesap ayarları (⋯)

- Ad, açılacak oyun (League/TFT, VALORANT, ikisi ya da yalnızca Riot Client), kısayol tuşu. "League ve VALORANT" seçilen hesabın satırında iki oyun için ayrı düğme çıkar; tepsi, kısayol tuşu ve masaüstü kısayolu League'i açar.
- Otomatik giriş: şifre kaydedersen, oturumun süresi dolduğunda giriş ekranına kendisi yazar.
- Masaüstü kısayolu, hesabı silme.

## Listede görünenler

- Riot ID, tek/çift sıralama ligi ve seviye: League istemcisi o hesapla açıkken okunur ve hatırlanır.
- **N gündür açılmadı:** hesabın oturumu en az 7 gündür kullanılmadı. Riot kullanılmayan oturumları bir süre sonra sonlandırır; süreyi açıklamaz. Hesaba geçince oturum hâlâ geçerliyse uyarı kalkar. Eşik `config.json` içindeki `stale_after_days` ile değişir, 0 uyarıyı kapatır.
- **giriş gerekli:** kayıtlı oturum son denemede kabul edilmedi. Şifre kayıtlıysa otomatik girilir; değilse Riot Client'ta "Oturumu açık tut" ile bir kez elle gir, kayıt kendiliğinden yenilenir.

**Ayarlar** (sağ alttaki dişli): sürüm ve güncellemeler, Windows açılınca tepside başlatma, pencereyi açan kısayol, güvenlik ve veriler.

## Riot Client'ı elle kullanırken

- Hesap değiştirmek için Riot Client'taki **Çıkış yap** yerine QuickSwitch'i kullan. Çıkış yapmak o oturumu Riot tarafında geçersiz kılabilir; kayıtlı kopya da onunla birlikte işe yaramaz hale gelir.
- Riot Client'ta elle başka bir hesaba girersen QuickSwitch bunu fark eder: hesap kayıtlıysa oturum o hesabın kaydına yazılır, kayıtlı değilse hiçbir kaydın üstüne yazılmaz ve "kaydet" önerisi çıkar.
- Bir kayıttan beklenenden farklı bir hesap açılırsa o kayıt silinir ve hesap "giriş gerekli" olarak işaretlenir.

## Otomatik giriş hakkında

Şifre, Riot Client'ın giriş penceresine klavye girişi olarak yazılır. Her adımdan önce öndeki pencerenin Riot Client olduğu denetlenir; değilse yazma durur. Doğrulama kodu (2FA) ya da captcha çıkarsa onu senin tamamlaman gerekir. Bu yolla açılan oturum "Oturumu açık tut" işaretlenmeden açıldığı için saklanmaz; o hesap her geçişte şifreyle girer.

## Güvenlik

- Veriler `%LOCALAPPDATA%\QuickSwitch\` içinde, Windows hesabına bağlı şifreli durur. Uygulamanın tek internet bağlantısı, açıksa güncelleme denetimidir.
- İsteğe bağlı **uygulama kilidi** oturumları ve şifreleri ayrıca bir ana şifreyle şifreler; uygulama her başladığında bir kez sorulur. Kilit, uygulama 15 dakika kullanılmayınca kendiliğinden yeniden kapanır (Güvenlik ekranından değiştirilir).
- Uygulamadaki **Güvenlik** ekranı neyin saklandığını gösterir, klasörü açar, kilidi yönetir ve tüm verileri siler.
- Ayrıntılar ve sınırlar: [SECURITY.md](SECURITY.md).

Riot bir güncellemede dosya ya da süreç adlarını değiştirirse `config.json` içinden düzeltilir (`session_paths`, `client_processes`, `game_processes`, `ui_processes`, `riot_client_exe`).

## Yapı

- `core/`: bütün mantık, arayüzden bağımsız. Riot Client'ın yerel arayüzü, süreçler ve klavye birer arayüzün arkasında durur; testler bunların sahteleriyle çalışır.
- `src-tauri/`: pencere, tepsi, kısayollar, arka plan döngüsü.
- `ui/`: düz HTML/CSS/JS, derleme adımı yok.

## Yasal

QuickSwitch, Riot Games tarafından onaylanmamıştır ve Riot Games ya da Riot Games mülklerinin üretiminde ya da yönetiminde resmî olarak yer alan kimseyin görüşlerini yansıtmaz. Riot Games, League of Legends ve VALORANT, Riot Games, Inc.'in ticari markalarıdır.

Kaynak kod, ne yaptığını herkes görebilsin diye açıktır. Bir lisansla dağıtılmaz: tüm hakları saklıdır. Kodu incelemek serbesttir; kopyalamak, değiştirmek ya da dağıtmak için izin gerekir.

