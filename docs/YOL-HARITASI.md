# QuickSwitch yol haritası: League Accounts'tan alınacaklar ve onu geçmek

Durum tarihi: 18 Kasım 2026. Bu belge 0.5.0 ile birlikte depoya girdi; 4. bölüm neyin yapıldığını, neyin kaldığını söyler.
Karşılaştırılan sürüm: [Tariolle/LeagueAccounts](https://github.com/Tariolle/LeagueAccounts) v3.5.0 (`cb2a30d`), kaynak kodu okunarak.

Bu belgedeki "onlar şunu yapıyor" cümleleri kodlarından okundu. Riot Client'ın nasıl davrandığına dair olanlar ise onların koduna dayanan varsayımlardır; biz doğrulamadık. Her işin yanında neyin burada test edilebildiği, neyin ancak gerçek istemcide görüleceği yazıyor.

## 1. İki uygulama nerede duruyor

| Konu | QuickSwitch | League Accounts |
|---|---|---|
| Giriş yöntemi | Kayıtlı oturumu yerine koyar; şifre gerekmez, captcha çıkmaz (ölçülen: 1,5 sn) | Her girişte şifreyle girer; captcha ve hız sınırına açık |
| Şifreyle giriş (yedek yol) | UI Automation, olmazsa klavye; gerçek istemcide henüz doğrulanmadı | Riot'un giriş sayfasını içeriden sürer; pencere gizli, durumları tek tek bilir |
| Giriş hatasını anlama | Tek mesaj: "şifre yanlış olabilir ya da doğrulama istiyor" | Yanlış şifre, captcha reddi, hız sınırı, doğrulama, hesap uyuşmazlığı, yönetici olarak çalışan istemci ayrı ayrı |
| Ban | Yok | Girişte okur, kalan süreyi gösterir, oyunu başlatmaz |
| Bölge | Denetlenmiyor | Oyundan önce istemcinin bölgesini hesabınkine çevirir |
| Saklama | Her şey DPAPI ile şifreli, isteğe bağlı ana şifre, kendiliğinden kilit | Şifreler Windows Credential Manager'da, hesap listesi dosyada |
| Veri taşıma | Yalnızca içe aktarma (0.4.5) | İçe ve dışa aktarma, toplu ekleme; dışa aktarma açık şifreli |
| Rank | League istemcisinden, yalnızca giriş yapılan hesap için | OP.GG'den, giriş yapmadan, arka planda; duo filtresi |
| Oyunlar | LoL, TFT, VALORANT | LoL, TFT |
| Koruma | Maçta ve kuyrukta geçişi engeller, "bitince geç" | Açık istemciyi kapatmadan önce sorar |
| Hızlı erişim | Hesap başına global kısayol, masaüstü kısayolu, tepsi | Uygulama içi kısayollar |
| Liste | Sürükleyerek sıralama, oyun filtresi, arama | Rank/ad/seviye/bölgeye göre sıralama, kart ve liste görünümü, not |
| İnternet | Yalnızca güncelleme denetimi | OP.GG ve güncelleme |
| Güncelleme | İmzalı, tek kanal, geri dönüş yok | Yeni sürüm "sağlıklı" bildirene kadar eskisini saklar |
| Testler (0.4.5'te) | Çekirdek uçtan uca (68), arayüz testi yok | Çekirdek, arayüz, kayıt gizliliği, CSP, güncelleme |
| Dil | Türkçe | 6 dil |

Özet: temel yöntemimiz daha iyi (şifresiz, hızlı, captcha'sız) ve saklama modelimiz daha sıkı. Onlar şifreli girişin sağlamlığında, hesap hakkında gösterilen bilgide, veri taşımada ve test genişliğinde önde.

## 2. İlke değişikliği: şifre gösterilebilir, ama bilerek

Bugüne kadar "kaydedilen şifre bir daha gösterilmez" diyorduk. Bundan sonra kural şu: **şifre, kullanıcı açıkça istediğinde ve ne olduğunu gördükten sonra uygulamadan çıkabilir.** Bunu onlardan daha iyi yapmanın yolu, çıkışın her biçimini ayrı ayrı korumak:

- **Şifreyi kopyala.** Onay ister; uygulama kilidi açıksa ana şifreyi yeniden sorar. Pano kaydı Windows pano geçmişine ve bulut eşitlemesine girmeyecek biçimde yazılır, 30 saniye sonra silinir (yalnızca panoda hâlâ bizim yazdığımız duruyorsa). Onlar da böyle yapıyor; bizdeki fark ana şifre adımı.
- **Şifreli yedek (varsayılan dışa aktarma).** Kullanıcının belirlediği parolayla, kilitte kullandığımız Argon2id + XChaCha20-Poly1305 ile şifrelenmiş tek dosya. Hesaplar, şifreler, kısayollar, notlar. Onlarda bunun karşılığı yok.
- **Açık dışa aktarma (uyumluluk için).** League Accounts'un okuyabildiği JSON. Ayrı düğme, kırmızı uyarı, "şifreler açık yazılacak" onayı. Bu, bizden onlara geçişi de mümkün kılar.

`SECURITY.md` ve Güvenlik ekranındaki "şifre bir daha ekranda gösterilmez" cümlesi bu sürümle birlikte değişmeli.

## 3. İşler

Büyüklük: K (bir oturum), O (birkaç oturum), B (denemeli, birden fazla sürüm).
Doğrulama: **burada** = sahte istemciyle test edilebilir; **sende** = gerçek Riot Client gerekir.

### A. Şifreli girişin sağlamlığı

**A1. Giriş hatalarını ayırmak** (K, kısmen burada)
Onlar: sonucu giriş sayfasının içinden okuyor. Biz sayfanın içinde değiliz, ama yerel arayüzdeki `/rso-authenticator/v1/authentication` ucu doğrulama kodu istendiğini (`type: multifactor`) ve hata metnini veriyor.
Yapılacak: şifreli girişten sonra bu ucu yoklayıp dört ayrı mesaj vermek: şifre reddedildi, doğrulama kodu bekleniyor, çok fazla deneme, captcha. Şifre reddedildiyse hesabı "şifre yanlış" diye işaretlemek ve yeniden denememek.
Daha iyi olan: doğrulama kodu beklenirken pencereyi öne getirip kullanıcıya haber vermek, kod girilince oturumu kendiliğinden kaydetmek.

**A2. Yönetici olarak çalışan Riot Client** (K, burada)
Onlar: başlatma hatası 740 ise ayrı mesaj veriyor. Bizde bu durumda geçiş anlaşılmaz biçimde başarısız olur. Aynı denetimi eklemek.

**A3. İlerleme adımları ve iptal** (K, burada)
Onlar: "istemci kapatılıyor, giriş yapılıyor, doğrulama, oyun başlatılıyor" adımlarını gösteriyor, iptal edilebiliyor.
Yapılacak: satırdaki bekleme göstergesinin yanına adım metni ve İptal. Bugünkü logda gördüğümüz "takıldı sanıp yeniden tıklama" davranışını da çözer.

**A4. Bölge denetimi** (O, sende) — olası hata
Onlar: hesabın League bölgesi istemcinin bölgesiyle uyuşmuyorsa oyundan önce `PUT /riotclient/region-locale` ile düzeltiyor.
Bizde: bölge ayarı `Riot Client/Config` içinde duruyor ve o klasöre bilerek dokunmuyoruz. Farklı bölgelerde hesabı olan biri için oyunun yanlış bölgeyle açılması mümkün. Önce doğrulanmalı: farklı bölgeden iki hesabın varsa birinden diğerine geçince League doğru sunucuya bağlanıyor mu?
Daha iyi olan: bölgeyi hesap kaydında tutup geçişte baştan doğru ayarlamak.

**A5. Giriş sayfasını içeriden sürmek** (B, sende) — koşullu
Onlar: Riot Client arayüzünü bir hata ayıklama kapısıyla kendileri başlatıp sayfada küçük bir betik çalıştırıyor. Klavye yok, odak yok.
Karar: 0.4.4'teki UI Automation yolunun logu gelince verilecek. Sayfa onu kabul ediyorsa bu işe gerek yok. Etmiyorsa bu yöntem, "Oturumu açık tut" işaretlemesiyle birlikte yazılır; böylece onlardan farklı olarak şifre hesap başına bir kez kullanılır.
Bedeli: giriş süresince yerel bir hata ayıklama kapısı açık kalır (yalnızca 127.0.0.1, rastgele kapı, giriş bitince kapanır) ve Riot'un iç yapısına bağımlılık artar. Kodları MIT lisanslı; betikten parça alınırsa telif notu korunmalı.

### B. Hesap hakkında bilgi

**B1. Ban denetimi** (K, burada + sende)
Onlar: girişten sonra kimlik bilgisindeki `ban.restrictions` listesinden League banını okuyor (süreli ya da kalıcı), oyunu başlatmıyor.
Bizde bu bilgi zaten çağırdığımız uçtan geliyor.
Daha iyi olan: banı hesap kaydında tutup listede giriş yapmadan da geri sayım göstermek; süre dolunca kendiliğinden kaldırmak; VALORANT kısıtlamalarını da okumak (alan adları loga yazılıp görüldükten sonra).

**B2. Not, sıralama, sıkışık görünüm** (O, burada)
Hesap başına kısa not; ada, ranka, seviyeye ve son kullanıma göre sıralama; çok hesap için tek satırlık liste. Arama notları da tarar. Elle sıralama varsayılan kalır.

**B3. Giriş yapmadan rank ve duo filtresi** (O, sende) — isteğe bağlı
Onlar: OP.GG'den çekiyor. Bizde "uygulama internete bağlanmaz" sözü var.
Yapılacak, istenirse: varsayılanı kapalı bir ayar; açılınca Güvenlik ekranında hangi adrese ne gönderildiği (yalnızca Riot ID ve bölge) yazar. Giriş yapılan hesapta League istemcisinden okunan değer her zaman önceliklidir.

### C. Veri taşıma

**C1. Toplu ekleme** (K, burada)
Satır başına `kullanıcı--Riot ID--şifre` ya da `kullanıcı--şifre`; onların biçimiyle aynı, böylece ellerindeki listeler doğrudan yapıştırılır. Eklemeden önce kaç hesabın ekleneceğini ve hangilerinin atlanacağını gösteren önizleme. Onlarda önizleme yok.

**C2. Şifreli yedek ve geri yükleme** (O, burada)
Bölüm 2'deki dosya. İçe aktarma kullanıcı adına ve hesap kimliğine göre eşleştirir, çakışanı atlar ya da üzerine yazar. Oturumlar isteğe bağlı eklenir; başka bilgisayarda Riot'un onları kabul edip etmediği denenmeden söz verilmez.

**C3. Açık dışa aktarma ve şifre kopyalama** (K, burada + sende)
Bölüm 2'deki koruma adımlarıyla. Pano davranışı gerçek Windows'ta denenmeli.

### D. Yayın ve test

**D1. Arayüz testleri** (O, burada)
Arayüzü zaten her değişiklikte sahte verilerle başsız tarayıcıda elle deniyorum. Bunu depoya girecek bir test dosyasına çevirip Actions'ta çalıştırmak: liste, ekleme sihirbazı, ayarlar, güvenlik, içe aktarma, kilit ekranı.

**D2. Kayıt gizliliği testi** (K, burada)
Onlar: loga yalnızca sabit olay kodları yazıyor. Bizde okunabilir cümleler var ve gizlilik yazarın dikkatine bağlı.
Yapılacak: bütün akışları belirgin sahte kullanıcı adı, şifre, token ve hesap adıyla çalıştırıp logda hiçbirinin geçmediğini doğrulayan test. Okunabilir logu koruyup onların güvencesini almış oluruz.

**D3. Erken sürüm kanalı** (O, sende)
Bugün her commit herkese gidiyor. Yapılacak: "Erken sürümleri al" ayarı. Yeni sürüm önce ön yayın olarak çıkar, yalnızca bu ayarı açanlara gider; sen deneyip onaylayınca herkese açılır. "Kimse bilmiyor" durumu değişince gerekecek.

**D4. Güncelleme sonrası sağlık denetimi** (O, sende)
Onlar: yeni sürüm açılıp "hazırım" diyene kadar eski dosyayı saklıyor.
Bizde kurulum dosyası kullanıldığı için aynı yöntem birebir uymaz. Karşılığı: güncellemeden önce önceki kurulum dosyasını saklamak; yeni sürüm iki kez üst üste açılamazsa bir sonraki açılışta "önceki sürüme dön" önermek.

## 4. Durum

0.5.0 ile tek pakette çıkanlar: geniş pencere ve yeni arayüz (ray, kartlar, geçiş hattı, tanı kaydı ekranı), A1, A2, A3, B1 (League banı; VALORANT sonra), B2, C1, C2, C3, D2 ve D1'in ilk hali (`tests/ui/test_ui.py`; henüz Actions'ta koşmuyor, elle çalıştırılıyor).

| Kalan | Neyi bekliyor |
|---|---|
| A4 bölge denetimi | Farklı bölgelerden iki hesapla gerçek istemcide deneme |
| A5 giriş sayfasını içeriden sürmek | 0.4.4'teki klavyesiz girişin gerçek logu |
| B3 giriş yapmadan rank, duo filtresi | "İnternete bağlanmaz" sözünden vazgeçme kararı |
| D1 arayüz testlerini Actions'a bağlamak | İş akışı dosyasını değiştirme izni olan bir oturum |
| D3 erken sürüm kanalı, D4 sağlık denetimi | Kullanıcı sayısı artınca |

## 5. Bilerek alınmayanlar

- **Her girişte şifreyle girmek.** Onların ana yöntemi; bizim oturum yöntemimiz daha hızlı ve captcha'ya takılmıyor. Şifre bizde yedek yol olarak kalır.
- **Şifreleri Credential Manager'a taşımak.** Aynı Windows kullanıcısına karşı DPAPI ile aynı korumayı verir, ana şifre katmanımızı ise desteklemez.
- **Yalnızca sabit kodlu kayıt.** Bugün logu okuyarak üç sorunu bir bakışta ayırabildik; D2 ile aynı güvence okunabilirlik kaybetmeden sağlanır.
- **Deceive uyumu, oyun dili ve bölgeye özel yamalar.** İsteyen olursa bakılır.

## 6. Açık sorular

1. 0.4.4'te şifreli hesapla giriş logu: UI Automation kabul edildi mi? (A5'in kaderi)
2. Farklı bölgelerde hesabın var mı, geçişte sunucu doğru mu? (A4)
3. OP.GG'den rank isteniyor mu, yoksa "internete bağlanmaz" sözü daha mı değerli? (B3)
4. Arkadaşın 0.4.5 ile içe aktarmayı denedi mi, dosya sorunsuz okundu mu?
