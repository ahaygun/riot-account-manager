# QuickSwitch

Riot Client hesapları arasında şifre yazmadan geçiş. Her hesabın "Oturumu açık tut" oturumunu saklar, seçtiğin hesabınkini yerine koyar ve oyunu başlatır. İstersen hesaplarının güncel League rankını da gösterir.

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

- Riot ID, tek/çift sıralama ligi ve seviye: League istemcisi o hesapla açıkken okunur ve hatırlanır. Güncel rank açıksa (aşağıda) hesaba girmeden de güncellenir; galibiyet, mağlubiyet ve kazanma oranı da görünür. Rankın üstüne gelince nereden ve ne zaman alındığı yazar.
- **Profil simgesi:** hesabın League'deki simgesi adının yanında gösterilir.
- **Rank amblemi:** her lig, League istemcisinin kendi amblemiyle gösterilir.
- **Riot ID bulunamadı:** OP.GG'de bu Riot ID bu bölgede yok. Hesap ayarlarından Riot ID'yi ve bölgeyi düzelt.
- **N gündür açılmadı:** hesabın oturumu en az 7 gündür kullanılmadı. Riot kullanılmayan oturumları bir süre sonra sonlandırır; süreyi açıklamaz. Hesaba geçince oturum hâlâ geçerliyse uyarı kalkar. Eşik `config.json` içindeki `stale_after_days` ile değişir, 0 uyarıyı kapatır.
- **ban: N gün / kalıcı ban:** Riot son girişte bu hesap için League banı bildirdi. Giriş yine yapılır; ban bitene kadar oyun açılırsa bağlantı kesilir. Hesaba banı kalkmış halde girince etiket kalkar.
- **Not:** hesap ayarlarında yazdığın kısa not satırın altında görünür; arama notlarda da arar.
- **Sıra ve görünüm:** hesaplar ada, ranka, seviyeye ya da son kullanıma göre gösterilebilir; sürükleyerek sıralama yalnızca "Kendi sıram" seçiliyken çalışır. Görünüm kartlar ya da satırlar olarak seçilir.
- **Soldaki sayılar:** giriş gerektiren, banlı ve uzun süredir açılmayan hesapları sayar; bir sayıya tıklamak listeyi ona indirir.
- **giriş gerekli:** kayıtlı oturum son denemede kabul edilmedi. Şifre kayıtlıysa otomatik girilir; değilse Riot Client'ta "Oturumu açık tut" ile bir kez elle gir, kayıt kendiliğinden yenilenir.

## Güncel rank ve internet

Uygulama ilk açılışta bir kez sorar: hesaplarının güncel rankını göstereyim mi? **Aç** dersen her League hesabının herkese açık [OP.GG](https://op.gg) sayfasına Riot ID'si ve bölgesiyle bakar, profil simgelerini Riot'un resim sunucusundan alır. **Kapalı kalsın** dersen hiçbir şey değişmez; uygulama eskisi gibi yalnızca güncelleme denetimi yapar.

- Rank kendiliğinden yenilenir (varsayılan: hesap başına 30 dakikada bir). Üstteki yenile düğmesi hepsine şimdi bakar; bir hesaba sağ tıklayıp **Rankı yenile** yalnızca ona bakar.
- Riot ID ve bölge, hesaba bir kez League ile girilince öğrenilir. İstersen hesap ayarlarından elle de yazabilirsin; böylece hiç girmediğin hesabın rankı da görünür.
- League istemcisi o hesapla açıkken okunan rank, OP.GG'den gelen daha eski bilginin önündedir.
- VALORANT rankı henüz gösterilmez.
- Soldaki **İnternet** bölümü her işin anahtarını, neyin nereye gittiğini ve **bağlantı kaydını** gösterir: uygulamanın kurduğu ve kurmayı reddettiği her bağlantı satır satır oradadır. Ana anahtarı kapatırsan uygulama hiçbir bağlantı kurmaz.
- Şifreler, kullanıcı adları ve oturumlar hiçbir bağlantıda gönderilmez. Ayrıntı: [SECURITY.md](SECURITY.md).

Bir hesaba **sağ tıklayınca** kısa bir menü açılır: hesaba geç, rankı yenile, Riot ID'yi kopyala, OP.GG'de aç, hesap ayarları.

## Pencere

Solda bölümler durur: **Hesaplar**, **Taşı ve yedekle**, **Güvenlik**, **İnternet**, **Tanı kaydı**, **Ayarlar**. Altında kilit açıksa kilitleme düğmesi ve sürüm.

Bir hesaba geçerken pencerenin altında **geçiş hattı** belirir: Riot Client'ın kapanması, oturumun yerine konması, istemcinin açılması, girişin kabulü, gerekiyorsa şifreyle giriş ve oyunun açılması birer durak olarak görünür. Geçilen duraklar yeşil, o anki durak sarıdır; bir şey durursa hangi durakta durduğu kırmızı kalır. Giriş takibi **İptal** ile bırakılabilir.

**Tanı kaydı** bölümü `quickswitch.log` dosyasının son satırlarını saatleriyle gösterir, sorun bildiren satırları işaretler ve tek tıkla panoya kopyalar. Bir şey çalışmadığında paylaşılacak şey budur. **Tanı paketi kaydet** daha ayrıntılı bir dosya yazar: Riot Client'ın o anki cevaplarının biçimi, oturum dosyalarının boyutu, ayarlar. İçinde hesap adı, kullanıcı adı, şifre ya da oturum yoktur; uygulama dosyayı hiçbir yere göndermez.

**Ayarlar:** sürüm ve güncellemeler, Windows açılınca tepside başlatma, pencereyi açan kısayol.

## Taşıma ve yedek

Soldaki **Taşı ve yedekle** bölümünde:

- **Şifreli yedek:** bütün hesaplar tek dosyaya, belirlediğin parolayla şifreli yazılır; başka bilgisayarda geri yüklenir. Kayıtlı oturumlar taşınmaz: şifresi kayıtlı hesaplar ilk geçişte şifreyle girer, diğerlerine bir kez elle giriş yaparsın.
- **Toplu ekle:** her satıra bir hesap, `kullanıcıadı--şifre` ya da `kullanıcıadı--Ad#ETİKET--şifre`. Eklemeden önce kaç hesabın ekleneceği gösterilir.
- **Açık dışa aktarma:** şifresi kayıtlı hesapları League Accounts'un okuduğu JSON olarak yazar. Şifreler açık yazılır; yalnızca taşımak için kullan ve işin bitince dosyayı sil.

Hesap ayarlarında (⋯) kayıtlı şifreyi panoya kopyalayabilirsin; pano geçmişine girmez ve 30 saniye sonra silinir.

## League Accounts'tan geçiş

[League Accounts](https://github.com/Tariolle/LeagueAccounts) kullanan biri hesaplarını tek seferde taşıyabilir: orada dışa aktar, burada **Taşı ve yedekle → League Accounts'tan içe aktar** ile JSON dosyasını seç. Hesaplar şifreyle giren hesaplar olarak eklenir; ilk girişten sonra kayıtlı oturumla açılırlar. Zaten kayıtlı kullanıcı adları atlanır. Dışa aktarma dosyasında şifreler açık yazılıdır; içe aktardıktan sonra sil.

## Riot Client'ı elle kullanırken

- Hesap değiştirmek için Riot Client'taki **Çıkış yap** yerine QuickSwitch'i kullan. Çıkış yapmak o oturumu Riot tarafında geçersiz kılabilir; kayıtlı kopya da onunla birlikte işe yaramaz hale gelir.
- Riot Client'ta elle başka bir hesaba girersen QuickSwitch bunu fark eder: hesap kayıtlıysa oturum o hesabın kaydına yazılır, kayıtlı değilse hiçbir kaydın üstüne yazılmaz ve "kaydet" önerisi çıkar.
- Bir kayıttan beklenenden farklı bir hesap açılırsa o kayıt silinir ve hesap "giriş gerekli" olarak işaretlenir.

## Otomatik giriş hakkında

Önce klavyesiz yol denenir: giriş alanları ve giriş düğmesi Riot Client penceresinin içinde bulunur, doldurulur ve düğmeye basılır. Pencerenin önde olması gerekmez; sen başka bir işle uğraşmaya devam edebilirsin. Bu sırada "Oturumu açık tut" da işaretlenir, böylece hesap bir kez şifreyle girer, sonraki geçişler kayıtlı oturumla olur.

Sayfa bu yolu kabul etmezse şifre klavye girişi olarak yazılır. O zaman pencerenin önde olması gerekir: her adımdan önce öndeki pencerenin Riot Client olduğu denetlenir, değilse yazma durur. Bu yolla açılan oturum saklanmaz.

Doğrulama kodu (2FA) ya da captcha çıkarsa onu senin tamamlaman gerekir.

## Güvenlik

- Veriler `%LOCALAPPDATA%\QuickSwitch\` içinde, Windows hesabına bağlı şifreli durur.
- İnternete yalnızca üç iş için çıkılır: güncelleme denetimi ve, sen açarsan, League rankı ile profil simgeleri. Her bağlantı yalnızca izinli sunuculara kurulur ve **İnternet** bölümündeki kayıtta görünür; hepsi tek anahtarla kapanır.
- İsteğe bağlı **uygulama kilidi** oturumları ve şifreleri ayrıca bir ana şifreyle şifreler; uygulama her başladığında bir kez sorulur. Kilit, uygulama 15 dakika kullanılmayınca kendiliğinden yeniden kapanır (Güvenlik ekranından değiştirilir).
- Uygulamadaki **Güvenlik** ekranı neyin saklandığını gösterir, klasörü açar, kilidi yönetir ve tüm verileri siler.
- Ayrıntılar ve sınırlar: [SECURITY.md](SECURITY.md).

Riot bir güncellemede dosya ya da süreç adlarını değiştirirse `config.json` içinden düzeltilir (`session_paths`, `client_processes`, `game_processes`, `ui_processes`, `riot_client_exe`).

## Yapı

- `core/`: bütün mantık, arayüzden bağımsız. Riot Client'ın yerel arayüzü, süreçler, klavye ve internet birer arayüzün arkasında durur; testler bunların sahteleriyle çalışır. İnternete çıkan her şey `core/src/net.rs` içindeki kapıdan geçer.
- `src-tauri/`: pencere, tepsi, kısayollar, arka plan döngüsü.
- `ui/`: düz HTML/CSS/JS, derleme adımı yok.

## Yasal

QuickSwitch, Riot Games tarafından onaylanmamıştır ve Riot Games ya da Riot Games mülklerinin üretiminde ya da yönetiminde resmî olarak yer alan kimseyin görüşlerini yansıtmaz. Riot Games, League of Legends ve VALORANT, Riot Games, Inc.'in ticari markalarıdır.

Rank amblemleri (`ui/ranks/` altındaki PNG dosyaları) Riot Games'e aittir; bütün hakları Riot Games'indir. League istemcisinin kendi dosyalarıdır ve değiştirilmeden paketlenir; her dosyanın alındığı adres, boyutu ve SHA-256 özeti `ui/ranks/manifest.json` içindedir, arayüz testi dosyaları bu listeyle karşılaştırır. QuickSwitch, Riot Games'in "Legal Jibber Jabber" politikasına dayanarak, Riot Games'e ait görseller kullanılarak hazırlanmıştır; Riot Games bu projeyi onaylamaz ve desteklemez. Bu izin Riot'un politikasındaki koşullara bağlıdır (ücretsiz ve ticari olmayan kullanım) ve Riot tarafından geri alınabilir; geri alınırsa amblemlerin kaldırılması gerekir. Uygulamadaki diğer her şey (simgeler, çerçeveler, arka plan) QuickSwitch'in kendi çizimidir; Riot'un yazı tipleri ve başka görselleri kullanılmaz.

Kaynak kod, ne yaptığını herkes görebilsin diye açıktır. Bir lisansla dağıtılmaz: tüm hakları saklıdır (Riot Games'e ait amblemler bunun dışındadır). Kodu incelemek serbesttir; kopyalamak, değiştirmek ya da dağıtmak için izin gerekir.

