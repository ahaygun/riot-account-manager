# QuickSwitch yol haritası (ikinci sürüm)

Durum tarihi: 18 Kasım 2026. Bu belge 0.6.0 ile birlikte güncellendi.

Bu belge, benzer on uygulamanın taranmasıyla çıkarılan fikirlerin tamamını, her biri için verilen kararı ve bunlara göre yeniden kurulan sürüm planını içerir. Önceki yol haritasının yerini alır.

## 1. Nelere bakıldı

| Uygulama | Nasıl incelendi | Öne çıkan yanı |
|---|---|---|
| [League Accounts](https://github.com/Tariolle/LeagueAccounts) v3.5.0 | Kaynak kodu ve arayüz metinlerinin tamamı | Şifreyle girişin sağlamlığı, ban, bölge, OP.GG rankları, duo filtresi |
| [Hextech Manager](https://github.com/MarceloVarela/league-account-manager) | README (ayrıntılı) | Hesap envanteri, "filo" görünümü, kasa güvenliği, Windows Hello |
| [League Account Switcher](https://github.com/Jonas-Merforth/league-account-switcher) | README | Gruplar, oyun içi yardımcılar, ayarları hesaplar arası taşıma |
| [Game Account Switcher](https://github.com/UnNooble/game-account-switcher) | README ve giriş betiği | UI Automation ile giriş, uygulama içinde 2FA, çökmeye dayanıklı dosya değişimi |
| [Riot Account Switcher](https://github.com/R-luiz/riot-account-switcher) | README | Windows Hello ile kilit |
| [accshift](https://github.com/klNuno/accshift) | README | Komut paleti, yayıncı modu, komut satırı ve bağlantı protokolü |
| [Arsenal](https://github.com/xfran1998/RiotAccManager) | README | Özel bölge etiketleri, geçiş adımları |
| [Riot Fast Login](https://github.com/bahaaalden2200-design/riot-fast-login) | README | Sihirdar simgesi, yeni oturumu kendiliğinden yakalama |
| [iMA Switcher](https://github.com/iMAboud/iMA-Switcher) | README özeti | VALORANT rankı, ayar eşitleme |
| [V-Shift](https://github.com/Nanda070/V-Shift), [RiotSwitcher](https://github.com/kizcntt/RiotSwitcher), [Alt Hop](https://github.com/giovaniohira/alt-hop) | README | Temalar, oyun dili, yeniden yakalama |

README'den aktarılanlar o uygulamaların kendi anlatımıdır; çalıştırıp doğrulamadım. Riot Client'ın davranışına dair her şey varsayımdır ve gerçek istemcide görülene kadar öyle kalır.

## 2. 0.5.2 itibarıyla elimizde olanlar

Oturum değiştirerek giriş (şifresiz, captcha'sız), şifreyle yedek giriş (UI Automation, olmazsa klavye), "Oturumu açık tut" işaretleme, giriş hatası ayrımı, League banı, geniş pencere, kart ve satır görünümü, rank armaları, geçiş hattı, tanı kaydı ekranı, şifreli yedek, toplu ekleme, League Accounts ile içe ve dışa aktarma, şifre kopyalama, uygulama kilidi ve kendiliğinden kilit, not, sıralama, durum filtreleri, hesap başına global kısayol, masaüstü kısayolu, "oyun bitince geç", maçta ve kuyrukta koruma, VALORANT.

Taranan uygulamaların hiçbirinde bunların hepsi bir arada yok. Eksik kaldığımız yerler aşağıda.

### 0.6.0 ile gelenler

Taramadan sonra verilen kararlarla (6. bölüm) uygulama "internete bağlanmaz" olmaktan çıkıp "internete yalnızca gösterdiği kadar bağlanır" oldu.

| Tablodaki madde | Ne geldi | Ne kaldı |
|---|---|---|
| B12 | Hesaba girmeden güncel League rankı, galibiyet/mağlubiyet ve seviye (OP.GG'nin herkese açık sayfasından), onayla ve kendiliğinden yenilenerek | Riot API anahtarı yolu alınmadı |
| B1 | Profil simgesi (Riot'un resim sunucusundan, diskte saklanarak) | |
| B2 | Tek/çift sırada galibiyet, mağlubiyet, kazanma oranı | Esnek sıra, geçen sezon, en yüksek lig |
| B8 | Bölge hesapta saklanıyor, istemciden okunuyor, elle değiştirilebiliyor | Bölgeye göre süzme, A4 |
| B9 | Riot ID'yi kopyalama, OP.GG'de açma | Porofessor, kullanıcı adını kopyalama |
| C1 | Sağ tık menüsü | Çift tıkla düzenleme |
| F1 | Tanı paketi: yerel uçların cevap biçimleri, oturum dosyalarının biçimi, ayarlar | Pencere ağacı dökümü |
| A1 | Alanı yazmadan önce odaklama | Düğme için ikinci yol |
| Yeni | İnternet kapısı (izinli sunucular, yalnızca HTTPS, yalnızca okuma), bağlantı kaydı, onay sorusu, ana anahtar | |

## 3. Fikir dökümü

Karar sütunu: **Al** (yapımıza uyuyor), **Uyarla** (fikri alıp kendi yolumuzla), **Karar** (senin tercihine bağlı), **Alma**.
Büyüklük: K (bir oturum), O (birkaç oturum), B (denemeli, birden fazla sürüm).

### A. Giriş ve geçiş

| # | Fikir | Kimde | Bizde | Karar | Büyüklük |
|---|---|---|---|---|---|
| A1 | UI Automation'da alanı yazmadan önce odaklamak; giriş düğmesi bulunamazsa şifre alanının altındaki ilk etkin düğmeyi seçmek | Game Account Switcher | Kısmen | **Al.** Aynı yöntemi çalıştıran bir uygulamanın kuralları; bizimkini sağlamlaştırır | K |
| A2 | Doğrulama kodunu uygulamada sorup Riot'un kutularına yazmak, kodun nereye gönderildiğini söylemek | Game Account Switcher | Yok | **Uyarla.** Geçiş hattına "Kod" durağı; A1 gerçek istemcide doğrulandıktan sonra | O |
| A3 | Giriş sayfasını içeriden sürmek | League Accounts | Yok | **Karar.** Yalnızca UI Automation gerçek istemcide çalışmazsa | B |
| A4 | Oyundan önce istemcinin bölgesini hesabınkine çevirmek | League Accounts, Arsenal | Yok | **Al.** Olası hatamız; önce farklı bölgeden iki hesapla doğrulanmalı | O |
| A5 | Hesap başına oyun dili | League Accounts | Yok | **Uyarla.** A4 ile aynı uç; isteğe bağlı alan | K |
| A6 | Oyun güncelleniyorsa ilerlemesini göstermek | League Accounts | Yok | **Al.** Geçiş hattına "Güncelleme" durağı, yüzdeyle | O |
| A7 | Başka hesapla açık League istemcisini kapatmadan önce sormak | League Accounts, Game Account Switcher | Kısmen (maç ve kuyruk engelli, lobi değil) | **Al.** Lobideyken onay iste; bekleyen açık sorumuzu kapatır | K |
| A8 | Banlı hesapta oyunu başlatmadan önce "yine de gir" onayı | League Accounts | Kısmen (yalnızca gösteriyoruz) | **Al** | K |
| A9 | Oyun açıldıktan sonra League'in doğru hesapta olduğunu doğrulamak | League Accounts | Kısmen (Riot Client'ta bakıyoruz) | **Al** | K |
| A10 | Dosya değişimini günlükleyip çökme sonrası geri almak | Game Account Switcher | Kısmen (yazımlar güvenli, değişim günlüksüz) | **Al.** Elektrik kesintisinde yarım kalmış değişimi sonraki açılışta toparlar | O |
| A11 | Tuş, kamera ve görüntü ayarlarını hesaplar arası taşımak | League Account Switcher, Hextech Manager, iMA Switcher, accshift | Yok | **Al.** Dört uygulamada var, en çok istenen yardımcı; yalnızca yerel dosyalar | O |
| A12 | Her yerden açılan hızlı geçiş paleti: küçük pencere, yaz ve Enter | Hextech Manager, accshift | Kısmen (pencere kısayolu var) | **Al.** Geniş pencere açılmadan geçiş | O |
| A13 | Bütün oturumları tek seferde sınamak | Game Account Switcher, League Account Switcher | Yok (yalnızca gün sayıyoruz) | **Karar.** Kayıtlı oturumla Riot'un sunucularına bağlanmayı gerektirir | O |
| A14 | Çevrimdışı görünmek | Game Account Switcher (Deceive ile), League Account Switcher, Hextech Manager | Yok | **Karar.** Deceive kuruluysa onun üzerinden başlatmak mümkün; Riot sohbeti değişince bozulabiliyor | O |
| A15 | Son iki hesap arasında tek tuşla geçiş | Hiçbirinde | Yok | **Al.** Bizim fikrimiz | K |

### B. Hesap bilgisi

Kural: bilgi önce League ya da Riot istemcisinden, o hesap açıkken okunur ve "son okunan" olarak saklanır. 0.6.0'dan beri, kullanıcı açtıysa, herkese açık kaynaklardan da güncellenir; istemciden okunan daha yeni bilgi her zaman önde gelir.

| # | Fikir | Kimde | Bizde | Karar | Büyüklük |
|---|---|---|---|---|---|
| B1 | Sihirdar simgesi | Riot Fast Login, Hextech Manager | Yok | **Uyarla.** League istemcisi açıkken simgeyi yerelden alıp saklamak; internete çıkmadan | O |
| B2 | Esnek sıra rankı, galibiyet ve mağlubiyet, geçen sezon, en yüksek lig | Hextech Manager, League Accounts | Yalnızca tek/çift | **Al** | K |
| B3 | TFT rankı ve LoL/TFT görünüm geçişi | League Accounts | Yok | **Al** | O |
| B4 | Mavi öz, RP, şampiyon ve kostüm sayısı | Hextech Manager | Yok | **Al** (sayılar); tam envanter D bölümünde | O |
| B5 | Son on maçın form çizgisi | Hextech Manager | Yok | **Uyarla.** Kartta küçük çizgi; sonraya | O |
| B6 | Duo filtresi: bir arkadaşın rankıyla oynayabilecek hesaplar | League Accounts | Yok | **Al.** Kayıtlı ranklar üzerinden; kurallar yazılırken doğrulanacak | K |
| B7 | Rank dağılımı | League Accounts | Yok | **Al.** Soldaki sayıların yanına | K |
| B8 | Bölge etiketi ve bölgeye göre süzme | League Accounts, Arsenal | Yalnızca içe aktarmada taşınıyor | **Al.** A4 ile birlikte | K |
| B9 | Kullanıcı adını ve Riot ID'yi kopyalamak, OP.GG ve Porofessor bağlantıları | League Accounts, League Account Switcher | Yok | **Al.** Bağlantı senin tarayıcında açılır, uygulama bağlanmaz | K |
| B10 | Hesap künyesi: açılış tarihi, Riot ID geçmişi, 2FA durumu, maskeli e-posta | Hextech Manager | Yok | **Uyarla.** Alan adları gerçek istemcide görüldükten sonra | O |
| B11 | Uzun süredir oynanmayan hesapta rank düşüşü uyarısı | Hextech Manager | Kısmen (oturum için) | **Uyarla.** Kurallar doğrulanınca | K |
| B12 | Giriş yapmadan rank (OP.GG ya da Riot API anahtarı) | League Accounts, Hextech Manager | Yok | **Karar.** "İnternete bağlanmaz" sözünü bozar | O |
| B13 | VALORANT rankı | iMA Switcher | Yok | **Karar.** Riot'un sunucularına hesabın tokenıyla bağlanmayı gerektirir | O |

### C. Arayüz ve düzen

| # | Fikir | Kimde | Bizde | Karar | Büyüklük |
|---|---|---|---|---|---|
| C1 | Sağ tık menüsü, çift tıkla düzenleme | League Accounts | Yok | **Al** | K |
| C2 | Kısayollar sayfası; ara, ekle, ayarlar, sil için tuşlar | League Accounts | Kısmen | **Al** | K |
| C3 | Hesapları gruplamak, grupları daraltmak (pencerede ve tepside) | League Account Switcher, Hextech Manager, accshift | Yok | **Al** | O |
| C4 | Arayüz boyutu | League Accounts | Yok | **Al** | K |
| C5 | Diller | League Accounts (6), accshift (7) | Türkçe | **Al.** Önce İngilizce; metinler tek dosyaya | O |
| C6 | Yayıncı modu: OBS açıkken adları gizlemek | Hextech Manager, accshift | Yok | **Al** | K |
| C7 | Geri alınabilir silme (çöp) | Hextech Manager | Yok | **Al.** Yanlışlıkla silinen hesabın şifresi ve oturumu geri gelir | K |
| C8 | Açık tema | Hextech Manager, V-Shift | Yok | **Uyarla.** Sonraya; renkler zaten değişkenlerde | O |
| C9 | Uygulama içi yardım | League Account Switcher | README | **Uyarla.** Boş durumlar ve ipuçlarıyla; ayrı sayfa değil | K |
| C10 | Riot Client konumunu arayüzden değiştirmek | Arsenal, Game Account Switcher | Yalnızca config.json | **Al** | K |
| C11 | Arayüz öğelerini gizleyip gösterme | iMA Switcher | Yok | **Alma.** Görünüm seçimi yeterli | |

### D. Çoklu hesap görünümü ("filo")

Tek hesaplı araçların yapamadığı şey: bütün hesaplara birden bakmak. Yalnızca Hextech Manager'da var.

| # | Fikir | Karar | Büyüklük |
|---|---|---|---|
| D1 | Bir kostümün ya da şampiyonun hangi hesaplarda olduğunu bulmak | **Al** | O |
| D2 | Toplam: şampiyon, kostüm, mavi öz, RP | **Al** | K |
| D3 | Hangi hesaplar şifresiz açılır, hangileri şifre ister | **Al.** Bizde bu bilgi zaten var | K |
| D4 | Hesap tablosunu CSV olarak vermek (şifresiz) | **Al** | K |
| D5 | Destek başvurusu için hesap özeti sayfası | **Uyarla.** B10'dan sonra | K |

### E. Güvenlik ve kasa

| # | Fikir | Kimde | Bizde | Karar | Büyüklük |
|---|---|---|---|---|---|
| E1 | Windows Hello ile kilit açmak; ana şifre her zaman geçerli kalır | Riot Account Switcher, Hextech Manager | Yok | **Al.** Kilidi kullanmayı kolaylaştırır | B |
| E2 | Windows kilitlenince uygulamayı da kilitlemek | Hextech Manager | Yalnızca boşta kalınca | **Al** | K |
| E3 | Aynı şifrenin kaç hesapta kullanıldığını söylemek | Hextech Manager | Yok | **Al.** Yerelde karşılaştırma, şifre gösterilmeden | K |
| E4 | Şifre geçmişi: yanlış yazılan yeni şifrede eskisine dönmek | Hextech Manager | Yok | **Al** | K |
| E5 | Kilit kapalıyken şifre çıkışı | Bizim açık sorumuz | Uyarıyla serbest | **Karar.** Kapatmak ya da Windows doğrulaması istemek | K |
| E6 | Yedek sağlığı: son yedek ne zaman, kasayla aynı diskte mi | Hextech Manager | Yok | **Al** | K |
| E7 | Kod imzalama | accshift | Yok | **Karar.** Ücretli sertifika | |

### F. Dayanıklılık ve bakım

| # | Fikir | Kimde | Karar | Büyüklük |
|---|---|---|---|---|
| F1 | **Tanı paketi:** Riot Client'ın ne sakladığını, pencere ağacını ve yerel uçların cevap biçimlerini değerler olmadan toplamak | Hextech Manager (lam-diag), Game Account Switcher (pencere dökümü), League Account Switcher (olay kaydedici) | **Al, ilk iş.** Gerçek istemciyi göremiyoruz; bu paket varsayımları olguya çevirir | O |
| F2 | Onarım: istemcileri kapat, bayat lockfile'ı sil | Hextech Manager | **Al.** Tanı kaydı sayfasına | K |
| F3 | Yedeğe oturumları da eklemek | Game Account Switcher | **Karar.** Başka bilgisayarda çalıştığı denenince | K |
| F4 | Kaldırırken veriyi de silmeyi sormak | Game Account Switcher | **Al** | K |
| F5 | Güncellemeyi arka planda indirip boştayken yeniden başlamak | League Accounts | **Uyarla.** İsteğe bağlı ayar | O |
| F6 | Yeni sürüm açılamazsa öncekine dönmek | League Accounts | **Al** | O |
| F7 | Erken sürüm kanalı | Bizim fikrimiz | **Al.** Kullanıcı sayısı artınca şart | O |
| F8 | Arayüz, CSP ve açılış testlerini Actions'ta koşturmak | League Accounts | **Al** | O |

### G. Oyun içi yardımcılar

Bunlar hesap değiştirmenin ötesine geçip League istemcisinin içinde iş yapar. Hextech Manager'ın kendi belgesi Riot'un kullanım koşullarının istemciyle etkileşen izinsiz programları yasakladığını not ediyor; hangisinin sorun sayıldığını doğrulamadım.

| # | Fikir | Kimde | Karar |
|---|---|---|---|
| G1 | Maç bulununca kendiliğinden kabul | League Account Switcher | **Karar.** Yaygın ama kapsam dışına çıkış; istersen ayrı ve kapalı gelen bir ayar |
| G2 | İstemci bildirimlerini temizlemek, ödülleri almak | League Account Switcher | **Alma.** Hesap değiştiricinin işi değil |
| G3 | Hesaplar arası birleşik arkadaş listesi ve sohbet | League Account Switcher | **Alma.** Arka planda birden çok hesaba Riot sunucularında oturum açar |

### H. Otomasyon

| # | Fikir | Kimde | Bizde | Karar | Büyüklük |
|---|---|---|---|---|---|
| H1 | Komut satırı: listele, geç, kilitle | accshift, V-Shift, Riot Fast Login | Yalnızca `--switch` | **Al** | K |
| H2 | Bağlantı protokolü (`quickswitch://`), Stream Deck için | accshift | Yok | **Al** | K |

## 4. Fazlamız: kimsede olmayanlar

1. **Görünen internet.** Rakipler rankı OP.GG'den ya da API anahtarıyla çekiyor ama neyin nereye gittiğini göstermiyor. Bizde her bağlantı tek bir kapıdan geçer, yalnızca izinli sunuculara gider, satır satır kayda yazılır ve tek anahtarla kapanır (0.6.0). Taranan uygulamaların hiçbirinde bağlantı kaydı yok.
2. **Tanı paketi ve uygulama içi kayıt.** Sorun çıktığında tek tıkla, sır içermeyen bir paket (F1). Rakiplerde ya ayrı bir araç ya da log klasörü var.
3. **Geçiş hattının tamamı.** Güncelleme ve doğrulama kodu duraklarıyla (A2, A6) bir geçişin her anı tek yerde görünür.
4. **İki kapılı kilit.** Ana şifre, Windows Hello ve şifre çıkışında yeniden sorma birlikte (E1, E2). Hextech Manager'da benzeri var, League Accounts'ta yok.
5. **Oturum önce, şifre sonra, VALORANT dahil.** League'e özel yöneticilerin çoğu VALORANT'ı desteklemiyor; oturum değiştiricilerin çoğu da rank ve ban göstermiyor.
6. **"Bitince geç" ve tek tuşla önceki hesap** (A15). Rakiplerde yok.

## 5. Sürüm planı

Sıra, neyin neye bağlı olduğuna göre kuruldu. 0.6.0 tek büyük yama olarak çıktı; sonrakiler onun getirdiği tanı paketinden gelecek bilgiye dayanıyor.

| Sürüm | Ad | İçerik | Neden bu sırada |
|---|---|---|---|
| **0.6.0** | İnternet | B12, B1, B2 (kısmen), B8 (kısmen), B9, C1, F1, A1, internet kapısı ve bağlantı kaydı | Yayında |
| **0.6.1** | Görünüm | League istemcisinin renkleri ve çerçeve dili: lacivert-siyah zemin, altın çerçeve, hextech turkuazı, büyük harfli serif başlıklar, köşesiz kutular. Hepsi CSS ile çizilir; pakete Riot'tan tek dosya girmez | İşlev değişmedi; uygulama istemcinin bir parçası gibi durunca daha çok güven veriyor |
| **0.6.2** | Amblemler | League istemcisinin özgün rank amblemleri: istemcinin kendi dosyaları, değiştirilmeden, kaynak adresi ve SHA-256 özetiyle (`ui/ranks/manifest.json`). Çizim armalar kaldırıldı; amblem rankın yanında, profil simgesi adın yanında. Ayarlar'da "Riot Games onaylamamıştır" notu | Arayüz istemciye benzeyince çizim armalar yabancı kaldı (karar 5) |
| 0.6.x | Düzeltmeler | OP.GG biçimi canlıda farklı çıkarsa uyarlama; tanı paketinden görülen farklar | Varsayımlar gerçek istemcide ve canlı sayfada ilk kez sınanıyor |
| 0.7.0 | VALORANT ve künye | B13 VALORANT rankı, B2'nin kalanı, B3 TFT, B4, B7 | VALORANT için gerçek istemcinin ne döndürdüğünü tanı paketinden görmek gerek |
| 0.7.1 | Düzen | C3 gruplar, C7 çöp, B6 duo filtresi, C6 yayıncı modu, A15, C2, C4 | Çok hesabı olanlar için |
| 0.8.0 | Geçiş | A4 bölge, A7, A8, A9, A6 güncelleme durağı, A10 günlüklü değişim, A2 uygulama içi kod, F2, C10 | Girişe dokunan her şey bir arada |
| 0.8.1 | Yardımcılar | A11 ayar taşıma, A12 hızlı geçiş paleti, H1, H2 | |
| 0.9.0 | Kasa ve filo | E1 Windows Hello, E2, E3, E4, E6, F4, D1 ile D5, B5, B10, B11 | Windows'a özel işler senin makinende deneme ister |
| 1.0.0 | Sağlamlık | F6, F7, F8, C5 İngilizce | Başkalarına açmadan önce |

Hiçbir sürüme yazılmayanlar: A3, A13, F3.

## 6. Verilen kararlar

1. **İnternet: evet.** Uygulama internete bağlanıp hesapların güncel halini gösterir. Karşılığında güvenlik hissi güçlendirildi: onay sorusu, izinli sunucu listesi, bağlantı kaydı, ana anahtar ([SECURITY.md](../SECURITY.md)). B12 yapıldı, B13 sırada.
2. **Kilit: zorlaştırılmayacak** (E5). Uygulamanın amacı rahat ve hızlı giriş; kilit kapalıyken şifre çıkışı bugünkü gibi uyarıyla serbest kalır.
3. **Oyun içi yardımcılar: alınmıyor** (G1, A14). Amaç dışı.
4. **Kod imzalama: beklemede** (E7). Ücretli olduğu için şimdilik yok.
5. **Rank amblemleri: Riot'un özgünleri.** 0.6.1'de arayüz League istemcisinin diline geçince çizim armalar yabancı kaldı; 0.6.2'den beri istemcinin kendi amblemleri pakette. Riot'tan alınan tek şey budur ve Riot'un topluluk projeleri politikasına (ücretsiz, ticari olmayan, bildirimli, geri alınabilir) dayanır; bildirim README'de ve Ayarlar'dadır.

## 7. Bilerek alınmayanlar

- **Her girişte şifreyle girmek** (League Accounts'un ana yolu): oturum yöntemimiz daha hızlı ve captcha'ya takılmıyor.
- **Yönetici olarak çalışmak** (Hextech Manager zorunlu tutuyor): gerekmiyor, saldırı yüzeyini büyütür.
- **Şifreleri Credential Manager'a taşımak:** ana şifre katmanımızı desteklemez.
- **Riot'un yazı tipleri, arka planları ve düğme görselleri** (League Accounts paketliyor): alınmıyor. Renkler ve çerçeveler 0.6.1'den beri League istemcisininkileri izliyor ama CSS ile çizilir; başlıklar Windows'ta zaten bulunan bir yazı tipiyle yazılır. Tek istisna rank amblemleridir (karar 5).
- **macOS ve Linux** (accshift): Riot Client'ın ve League'in olduğu yer Windows.
- **Bildirim temizleme, arkadaş listesi, sohbet** (G2, G3).
- **Kendiliğinden maç kabulü ve çevrimdışı görünme** (G1, A14): hesap değiştirmenin dışında.
- **Tarayıcı taklidi yaparak veri çekmek:** uygulama internette kendini adıyla tanıtır; bir site bunu reddederse o veri gösterilmez.
