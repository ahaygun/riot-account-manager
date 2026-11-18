# Güvenlik

Bu belge QuickSwitch'in neyi sakladığını, nasıl koruduğunu ve neye karşı koruyamadığını anlatır. Buradaki her madde kodda karşılığı olan bir davranıştır.

## Ne saklanır, nerede

Her şey `%LOCALAPPDATA%\QuickSwitch\` içinde durur. `Roaming` yerine `Local` seçildi: bu klasör Windows tarafından başka bilgisayarlara eşitlenmez.

| Dosya | İçerik | Koruma |
|---|---|---|
| `accounts.bin` | Hesap adları, Riot ID, son bilinen lig, ayarlar | DPAPI |
| `snapshots\*.bin` | "Oturumu açık tut" oturumları | DPAPI, kilit açıksa ayrıca ana şifre |
| `secrets\*.bin` | Kaydetmeyi seçtiğin kullanıcı adı ve şifreler | DPAPI, kilit açıksa ayrıca ana şifre |
| `vault.json` | Kilit için tuz, Argon2 ayarları ve doğrulama değeri | Ana şifreyi ya da anahtarı içermez |
| `config.json` | Dosya ve süreç adları, internet tercihlerin | Gizli veri içermez |
| `network.log` | Uygulamanın kurduğu ve kurmayı reddettiği her internet bağlantısı: zaman, amaç, sunucu, sonuç, boyut | Adresin yolu, Riot ID ya da hesap adı içermez; son 300 satır tutulur |
| `icons\*.png` | İnternetten alınmış profil simgeleri | Herkese açık resimler; gizli veri içermez |
| `quickswitch.log` | Sorun gidermek için kısa teknik kayıt (örneğin "pencere öne getirildi", "giriş kabul edildi") | Hesap adı, kullanıcı adı, şifre ya da oturum bilgisi içermez; 256 KB'ı geçince yenilenir |

## Şifreleme

- **Her zaman:** Windows DPAPI. Dosyaları yalnızca aynı bilgisayardaki aynı Windows kullanıcısı açabilir. Diski çıkarıp başka bir makinede okumak ya da dosyaları kopyalamak işe yaramaz.
- **Uygulama kilidi (isteğe bağlı):** Ana şifreden Argon2id (64 MiB, 3 tur) ile 256 bit anahtar türetilir; oturumlar ve şifreler XChaCha20-Poly1305 ile şifrelenir. Anahtar yalnızca bellekte, yalnızca uygulama açıkken tutulur. Ana şifre hiçbir yere yazılmaz ve kurtarılamaz.
- Değiştirilmiş bir dosya açılmaz; şifreleme bütünlüğü de doğrular.
- **Kendiliğinden kilitlenme:** Kilit açıkken uygulama bir süre (varsayılan 15 dakika) kullanılmazsa anahtar bellekten silinir ve ana şifre yeniden sorulur. Geçiş yapmak, pencerede tıklamak ya da yazmak kullanım sayılır. Süren bir giriş takibi ya da sıradaki bir geçiş varken kilitlenmez.
- Dosyalar önce geçici bir adla yazılır, diske ulaştığı doğrulanır, sonra asıl adını alır; elektrik kesintisi yarım dosya bırakmaz. Kilit değişimi yarıda kesilirse sonraki açılışta ya tamamlanır ya da geri alınır.

## İnternet

Hesap değiştirmek internet gerektirmez: oturum dosyaları bu bilgisayarda yer değiştirir. İnternet yalnızca aşağıdaki üç iş için kullanılır ve üçü de kapatılabilir.

### Ne gider, nereye gider

| Ne için | Nereye | Giden | Ne zaman |
|---|---|---|---|
| League rankı, galibiyet/mağlubiyet, seviye | `op.gg`, `www.op.gg` | Hesabın Riot ID'si ve bölgesi (adresin içinde). İkisi de oyundaki herkesin gördüğü bilgidir. | Yalnızca sen "Aç" dedikten sonra |
| Profil simgesi | `ddragon.leagueoflegends.com` (Riot'un resim sunucusu) | Simgenin numarası | Yalnızca sen "Aç" dedikten sonra |
| Güncelleme | `github.com`, `objects.githubusercontent.com`, `release-assets.githubusercontent.com` | Sana ait hiçbir şey; sürüm dosyası okunur | Her zaman olduğu gibi; Ayarlar'dan ya da İnternet bölümündeki ana anahtardan kapanır |

**Hiçbir bağlantıda gitmeyenler:** şifreler, kullanıcı adları, kayıtlı oturumlar, Riot hesap kimliği (PUUID), hesaba verdiğin ad, notlar, dosya yolları. Telemetri, analiz ve hata raporu yoktur.

### Sana sorulur, sonra da görürsün

- **Sorulur.** Rank ve simge için uygulama ilk açılışta bir kez sorar; "Aç" demeden bu iki iş için tek bir bağlantı kurulmaz. "Kapalı kalsın" dersen uygulama 0.5 sürümlerindeki gibi yalnızca güncelleme denetimi yapar.
- **Kapanır.** İnternet bölümünde her işin kendi anahtarı ve hepsinin üstünde bir ana anahtar vardır. Ana anahtar kapalıyken uygulama hiçbir bağlantı kurmaz; güncelleme denetimi de durur.
- **Görürsün.** Aynı bölümdeki **Bağlantı kaydı** kurulan her bağlantıyı ve reddedilen her isteği satır satır gösterir: ne zaman, ne için, hangi sunucuya, nasıl sonuçlandı, kaç bayt geldi. Kayıt `network.log` dosyasıdır; uygulama onu hiçbir yere göndermez.

### Bunu kod nasıl garanti eder

Rank ve simge bağlantılarının hepsi tek bir yerden geçer: `core/src/net.rs` içindeki kapı. Güncelleyici dışında internete çıkan başka kod yoktur; Riot Client'ın yerel arayüzüyle konuşan kod yalnızca `127.0.0.1` adresine bağlanır. Kapının kuralları:

- Her işin kendi sunucu listesi vardır; listede olmayan bir adres, bağlantı kurulmadan reddedilir ve kayda "reddedildi" diye geçer.
- Yalnızca HTTPS, yalnızca standart kapı; adresin içinde kullanıcı adı ya da şifre taşıyan biçimler reddedilir.
- Yalnızca okuma (GET) yapılır. Kapıyı çağıran kod isteğe başlık ya da gövde ekleyemez; yani bir bağlantıda gidebilecek tek şey adresin kendisi ve uygulamanın adıyla sürümüdür. Çerez tutulmaz.
- Yönlendirmeler kendiliğinden izlenmez: her adım aynı kurallardan yeniden geçer, en çok dört adım.
- Cevaplara boyut sınırı vardır (sayfa için 6 MB, resim için 512 KB); aşan cevap atılır.
- Anahtarı kapalı bir iş için bağlantı hiç kurulmaz.

Güncelleme denetimini Tauri'nin güncelleyicisi yapar, kapı değil; ama aynı ana anahtara uyar ve aynı kayda yazılır. Yalnızca imza anahtarıyla derlenmiş sürümlerde bulunur. Güncelleme ancak sen onaylarsan ve imzası uygulamaya gömülü açık anahtarla doğrulanırsa kurulur. Ayrıntı: [docs/UPDATES.md](docs/UPDATES.md).

Bu kuralların her biri testlidir (`core/src/net.rs` ve `core/tests/switching.rs`): sahte bir internet her isteği kaydeder ve testler onay verilmeden tek istek çıkmadığını, listede olmayan adrese gidilmediğini, bir yönlendirmenin başka sunucuya kaçamadığını ve isteklerde Riot ID dışında hesaba ait hiçbir şeyin geçmediğini doğrular.

### Pencere internete çıkamaz

Arayüz yalnızca uygulamayla gelen dosyaları yükler; içerik güvenlik ilkesi uzak kaynakların tamamını engeller. Profil simgeleri pencereye adres olarak değil, uygulamanın çekirdeğinin indirip denetlediği veri olarak gelir. "OP.GG'de aç" düğmesinde adresi pencere değil çekirdek, kayıtlı Riot ID'den kurar; pencere tarayıcıya kendi seçtiği bir adresi açtıramaz.

### İnternetten gelen veriye güvenilmez

- OP.GG sayfasından yalnızca bilinen lig adları ve sayılar okunur. Sayfa değişirse ya da yanlış bir şey döndürürse olabilecek en kötü şey yanlış ya da eksik bir rank görmektir; sayfadaki metin pencerede kod olarak çalışmaz.
- İndirilen resim PNG değilse ya da sınırdan büyükse atılır.
- League istemcisi o hesapla açıkken okunan rank, daha eski bir sayfa bilgisinin önündedir.

### Karşı taraf ne öğrenir

Bunları bilerek açmanı isteriz:

- Her internet bağlantısında olduğu gibi, bağlanılan sunucu IP adresini görür.
- OP.GG, aynı IP adresinden sorulan Riot ID'leri birlikte görür; bu hesapların aynı kişiye ait olabileceği sonucunu çıkarabilir. Bu senin için sorunsa rank almayı kapalı tut; simgeler ve güncelleme bunu açığa çıkarmaz.
- Uygulama kendini gizlemez: isteklerde adı ve sürümü yazar, tarayıcı taklidi yapmaz.
- İnternet sağlayıcın ya da ağ yöneticin yalnızca hangi sunucuya bağlandığını görür; hangi hesaba bakıldığını görmez (bağlantı şifrelidir).

Uygulama OP.GG'ye yük olmamaya çalışır: kendiliğinden bakarken dakikada en çok dört hesaba, her hesaba en sık senin seçtiğin aralıkta (varsayılan 30 dakika) bakar; sunucu "yavaşla" derse 15 dakika hiç sormaz.

### Sınır

OP.GG'nin resmî bir arayüzü yoktur; herkesin tarayıcıda gördüğü sayfa okunur. OP.GG bunu engelleyebilir ya da sayfasını değiştirebilir. O zaman rank güncellenmez ve bağlantı kaydı nedenini gösterir; hesap değiştirme bundan etkilenmez.

### Yerel bağlantılar

Uygulama `127.0.0.1` üzerindeki Riot Client ve League istemcisinin yerel arayüzüyle konuşur (giriş durumu, hesap kimliği, lig, düzgün kapatma). Bu istekler bilgisayardan çıkmaz ve sistem vekil sunucusu ayarlarını kullanmaz.

## Tanı paketi

Tanı kaydı bölümündeki **Tanı paketi kaydet**, sorun gidermek için bir dosya yazar. Dosya İndirilenler klasörüne yazılır ve uygulama onu hiçbir yere göndermez; paylaşıp paylaşmamak sana kalır. İçinde değerler değil **biçimler** bulunur: Riot Client'ın yerel arayüzünden gelen cevapların hangi alanları taşıdığı ve türleri, oturum dosyalarının boyutu ve anahtar adları, çalışan süreç adları, ayarlar, hesapların sayısı ve özellikleri (şifresi kayıtlı mı, oturumu var mı). Hesap adı, Riot ID, kullanıcı adı, şifre, token ve oturum içeriği pakete girmez; bunu da bir test denetler.

## Şifreler

- Şifre kaydetmek isteğe bağlıdır. Önerilen yol oturumla eklemektir; o yolda şifre hiç saklanmaz.
- Kaydedilen şifre arayüze geri gönderilmez ve gösterilmez. Uygulamadan yalnızca kullanıcının açık isteğiyle, aşağıdaki "Şifrenin uygulamadan çıktığı yollar" bölümündeki üç yoldan biriyle çıkar.
- Şifre önce Riot Client penceresindeki giriş alanına doğrudan (Windows UI Automation ile) yazılır. Pencere, adı eşleşen **ve** çalıştırılabilir dosyası Riot Client kurulum klasöründe olan bir sürece aittir; alanların da o süreçlere ait olduğu ayrıca denetlenir. Bu yolda klavye kullanılmaz, yani şifre başka bir pencereye gidemez.
- Sayfa bunu kabul etmezse alanlar boşaltılır ve şifre klavye girişi olarak yazılır. Yazmadan önce ve her adımda öndeki pencerenin aynı koşulları sağlayan bir sürece ait olduğu denetlenir. Değilse yazma durur.
- Şifre komut satırına ya da günlük dosyasına yazılmaz; bellekteki kopyası iş bitince silinir. Bunu bir test de denetler: bütün akışlar tanınabilir sahte ad, kullanıcı adı, şifre ve token ile çalıştırılır ve günlükte hiçbirinin geçmediği doğrulanır.

## Şifrenin uygulamadan çıktığı yollar

Üçü de yalnızca kullanıcı istediğinde çalışır. Uygulama kilidi açıksa üçü de, uygulama o an açık olsa bile, ana şifreyi yeniden sorar: başında durulmayan açık bir bilgisayarda hesap değiştirilebilir ama şifreler alınıp götürülemez.

- **Şifreyi kopyala.** Şifre arayüze uğramadan panoya yazılır. Pano kaydı Windows pano geçmişine ve cihazlar arası eşitlemeye girmeyecek biçimde işaretlenir ve 30 saniye sonra, araya başka bir kopyalama girmediyse silinir. O 30 saniye içinde panoyu okuyan her program şifreyi görebilir.
- **Şifreli yedek.** Bütün hesaplar, kayıtlı şifreleri, notları ve kısayollarıyla tek dosyaya yazılır. Dosya kullanıcının belirlediği parolayla şifrelenir (Argon2id ile anahtar, XChaCha20-Poly1305 ile şifreleme); Windows hesabına bağlı değildir, bu yüzden başka bilgisayarda açılır. Dosyanın güvenliği o parolanın gücü kadardır. Kayıtlı oturumlar yedeğe girmez.
- **Açık dışa aktarma.** League Accounts uygulamasının okuduğu JSON. **Şifreler bu dosyada açık yazılıdır.** Ayrı bir onay kutusu işaretlenmeden oluşturulmaz, dosya adı da ne olduğunu söyler. Yalnızca başka bir uygulamaya taşımak içindir.

İçe aktarma (League Accounts dosyası, toplu ekleme, yedekten geri yükleme) şifreleri doğrudan şifreli depoya yazar. Seçilen dosya uygulama tarafından silinmez; açık şifreli bir dosyayı silmek kullanıcıya kalır.

## Diğer önlemler

- Maç sürerken hesap değiştirilmez.
- Oturum dosyaları yalnızca Riot Client kapalıyken ve yalnızca tanımlı yollara yazılır; kayıt içindeki yol bu yolların dışına çıkıyorsa geri yükleme reddedilir.
- Açık bir oturum, Riot hesap kimliği eşleşmeyen bir hesabın üstüne kaydedilmez. Riot Client'ta elle yapılan girişler arka planda izlenir (yalnızca kimin girdiği, yerel arayüzden), böylece kapalı istemcinin bıraktığı oturum da yanlış hesaba yazılmaz. Uygulama çalışmıyorken elle başka hesaba girildiyse bu görülemez; o durumda yanlış kayıt ilk geçişte fark edilip silinir.
- Kilidi kaldırmak ya da değiştirmek, uygulama açıkken bile mevcut ana şifreyi ister.
- **Tüm verileri sil** her şeyi kaldırır; Riot Client'ın kendi dosyalarına dokunmaz.

## Neye karşı korumaz

- **Windows hesabını kullanan biri.** Kayıtlı oturum, şifresiz giriş demektir. Kilit kapalıyken bilgisayarın başına geçen herkes hesaplara geçebilir. Paylaşılan bilgisayarda uygulama kilidini aç.
- **Senin adına çalışan kötü amaçlı yazılım.** Böyle bir yazılım Riot Client'ın kendi oturum dosyasını (o an açık hesap) zaten okuyabilir ve tuş vuruşlarını kaydedebilir. Kilit kapalıyken QuickSwitch'in dosyalarını da açabilir. Kilit açıkken, uygulama kilitliyken diğer hesapların kapalı kalır; kilit açılınca aynı yazılım ana şifreyi tuş kaydıyla öğrenebilir.
- **Riot tarafı.** Bu resmi bir Riot aracı değildir. Hesap güvenliği için Riot'un iki adımlı doğrulamasını açık tut.

## Dağıtım

Kurulum dosyası şu an imzasızdır; Windows SmartScreen uyarı gösterebilir. Güven için kaynak koddan derlemek ya da yayınlanan dosyanın sağlama toplamını karşılaştırmak mümkündür.

## Açık bildirimi

Bir güvenlik sorunu bulursan herkese açık bir kayıt açmadan önce depo sahibine özel olarak bildir.
