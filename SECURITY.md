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

## Şifreleme

- **Her zaman:** Windows DPAPI. Dosyaları yalnızca aynı bilgisayardaki aynı Windows kullanıcısı açabilir. Diski çıkarıp başka bir makinede okumak ya da dosyaları kopyalamak işe yaramaz.
- **Uygulama kilidi (isteğe bağlı):** Ana şifreden Argon2id (64 MiB, 3 tur) ile 256 bit anahtar türetilir; oturumlar ve şifreler XChaCha20-Poly1305 ile şifrelenir. Anahtar yalnızca bellekte, yalnızca uygulama açıkken tutulur. Ana şifre hiçbir yere yazılmaz ve kurtarılamaz.
- Değiştirilmiş bir dosya açılmaz; şifreleme bütünlüğü de doğrular.

## Ağ

- Uygulama internete bağlanmaz. Telemetri, analiz, hata raporu yoktur.
- Tek ağ trafiği `127.0.0.1` üzerindeki Riot Client ve League istemcisinin yerel arayüzüne gider (giriş durumu, hesap kimliği, lig, düzgün kapatma). Bu istekler sistem vekil sunucusu ayarlarını kullanmaz.
- Arayüz yalnızca uygulamayla birlikte gelen dosyaları yükler; içerik güvenlik ilkesi uzak kaynakları engeller.
- Otomatik güncelleme eklendiğinde bu bölüm değişecek: tek dış bağlantı o olacak, kapatılabilecek ve güncellemeler imza doğrulamasından geçecek.

## Şifreler

- Şifre kaydetmek isteğe bağlıdır. Önerilen yol oturumla eklemektir; o yolda şifre hiç saklanmaz.
- Kaydedilen şifre arayüze geri gönderilmez ve gösterilmez.
- Şifre, Riot Client'ın giriş penceresine klavye girişi olarak yazılır. Yazmadan önce ve her adımda öndeki pencerenin, adı eşleşen **ve** çalıştırılabilir dosyası Riot Client kurulum klasöründe olan bir sürece ait olduğu denetlenir. Değilse yazma durur.
- Şifre komut satırına, günlük dosyasına ya da panoya yazılmaz; bellekteki kopyası iş bitince silinir.

## Diğer önlemler

- Maç sürerken hesap değiştirilmez.
- Oturum dosyaları yalnızca Riot Client kapalıyken ve yalnızca tanımlı yollara yazılır; kayıt içindeki yol bu yolların dışına çıkıyorsa geri yükleme reddedilir.
- Açık bir oturum, Riot hesap kimliği eşleşmeyen bir hesabın üstüne kaydedilmez.
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
