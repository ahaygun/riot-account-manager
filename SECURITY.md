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
| `config.json` | Dosya ve süreç adları | Gizli veri içermez |
| `quickswitch.log` | Sorun gidermek için kısa teknik kayıt (örneğin "pencere öne getirildi", "giriş kabul edildi") | Hesap adı, kullanıcı adı, şifre ya da oturum bilgisi içermez; 256 KB'ı geçince yenilenir |

## Şifreleme

- **Her zaman:** Windows DPAPI. Dosyaları yalnızca aynı bilgisayardaki aynı Windows kullanıcısı açabilir. Diski çıkarıp başka bir makinede okumak ya da dosyaları kopyalamak işe yaramaz.
- **Uygulama kilidi (isteğe bağlı):** Ana şifreden Argon2id (64 MiB, 3 tur) ile 256 bit anahtar türetilir; oturumlar ve şifreler XChaCha20-Poly1305 ile şifrelenir. Anahtar yalnızca bellekte, yalnızca uygulama açıkken tutulur. Ana şifre hiçbir yere yazılmaz ve kurtarılamaz.
- Değiştirilmiş bir dosya açılmaz; şifreleme bütünlüğü de doğrular.
- **Kendiliğinden kilitlenme:** Kilit açıkken uygulama bir süre (varsayılan 15 dakika) kullanılmazsa anahtar bellekten silinir ve ana şifre yeniden sorulur. Geçiş yapmak, pencerede tıklamak ya da yazmak kullanım sayılır. Süren bir giriş takibi ya da sıradaki bir geçiş varken kilitlenmez.
- Dosyalar önce geçici bir adla yazılır, diske ulaştığı doğrulanır, sonra asıl adını alır; elektrik kesintisi yarım dosya bırakmaz. Kilit değişimi yarıda kesilirse sonraki açılışta ya tamamlanır ya da geri alınır.

## Ağ

- Telemetri, analiz ve hata raporu yoktur.
- Uygulama `127.0.0.1` üzerindeki Riot Client ve League istemcisinin yerel arayüzüyle konuşur (giriş durumu, hesap kimliği, lig, düzgün kapatma). Bu istekler sistem vekil sunucusu ayarlarını kullanmaz.
- **Güncelleme denetimi** uygulamanın tek internet bağlantısıdır ve yalnızca imza anahtarıyla derlenmiş sürümlerde bulunur. GitHub'daki yayın sayfasından küçük bir sürüm dosyası indirir; hesaplarla ilgili hiçbir bilgi göndermez. Güvenlik ekranından kapatılabilir. Güncelleme ancak kullanıcı onaylarsa ve imzası uygulamaya gömülü açık anahtarla doğrulanırsa kurulur. Ayrıntı: [docs/UPDATES.md](docs/UPDATES.md).
- İmza anahtarı olmadan derlenmiş sürümlerde güncelleyici hiç yüklenmez ve uygulama internete bağlanmaz. Hangisinin kurulu olduğunu Güvenlik ekranının ilk maddesi söyler.
- Arayüz yalnızca uygulamayla birlikte gelen dosyaları yükler; içerik güvenlik ilkesi uzak kaynakları engeller.

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
