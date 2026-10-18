#import <Foundation/Foundation.h>
#import <UserNotifications/UserNotifications.h>
#import <stdatomic.h>
static atomic_int gjPermission=0; // unknown, notDetermined, denied, granted
static atomic_llong gjClick=0;
static atomic_int gjDispatchError=0;
static atomic_bool gjRefreshPending=false;
static atomic_bool gjRequestPending=false;
@interface GJNotificationDelegate : NSObject <UNUserNotificationCenterDelegate>
@end
@implementation GJNotificationDelegate
- (void)userNotificationCenter:(UNUserNotificationCenter *)center willPresentNotification:(UNNotification *)notification withCompletionHandler:(void (^)(UNNotificationPresentationOptions))completion {
    completion(UNNotificationPresentationOptionBanner|UNNotificationPresentationOptionList);
}
- (void)userNotificationCenter:(UNUserNotificationCenter *)center didReceiveNotificationResponse:(UNNotificationResponse *)response withCompletionHandler:(void (^)(void))completion {
    // Only our numeric identifier crosses the bridge, never a URL or source body.
    NSString *key=response.notification.request.identifier;
    if ([key hasPrefix:@"gitjuke-intent-"] && [response.actionIdentifier isEqualToString:UNNotificationDefaultActionIdentifier]) {
        NSString *suffix=[key substringFromIndex:15];
        NSScanner *scanner=[NSScanner scannerWithString:suffix];long long value=0;
        if ([scanner scanLongLong:&value] && scanner.isAtEnd && value>0) atomic_store(&gjClick,value);
    }
    completion();
}
@end
static GJNotificationDelegate *gjDelegate;
static UNUserNotificationCenter *gjCenter;
void gj_notifications_refresh(void) {
    if(!gjCenter)return;
    if(atomic_exchange(&gjRefreshPending,true))return;
    [gjCenter getNotificationSettingsWithCompletionHandler:^(UNNotificationSettings *s){
        int v=s.authorizationStatus==UNAuthorizationStatusNotDetermined?1:s.authorizationStatus==UNAuthorizationStatusDenied?2:(s.authorizationStatus==UNAuthorizationStatusAuthorized||s.authorizationStatus==UNAuthorizationStatusProvisional)?3:0;
        atomic_store(&gjPermission,v);atomic_store(&gjRefreshPending,false);
    }];
}
void gj_notifications_initialize(void) {
    // `tauri dev` runs a bare executable. UserNotifications raises an Objective-C
    // exception without an application bundle, which cannot cross the Rust FFI.
    NSBundle *bundle=[NSBundle mainBundle];
    if(![bundle.bundleURL.pathExtension isEqualToString:@"app"] || !bundle.bundleIdentifier.length)return;
    gjCenter=[UNUserNotificationCenter currentNotificationCenter];
    gjDelegate=[GJNotificationDelegate new];
    gjCenter.delegate=gjDelegate;
    gj_notifications_refresh();
}
int gj_notifications_permission(void) { return atomic_load(&gjPermission); }
void gj_notifications_request(void) {
    // The Rust command persists explicit opt-in before reaching this function.
    if(!gjCenter)return;
    if(atomic_exchange(&gjRequestPending,true))return;
    [gjCenter getNotificationSettingsWithCompletionHandler:^(UNNotificationSettings *settings){
        if(settings.authorizationStatus!=UNAuthorizationStatusNotDetermined){atomic_store(&gjRequestPending,false);gj_notifications_refresh();return;}
        [gjCenter requestAuthorizationWithOptions:UNAuthorizationOptionAlert completionHandler:^(BOOL granted,NSError *error){(void)granted;(void)error;atomic_store(&gjRequestPending,false);gj_notifications_refresh();}];
    }];
}
long long gj_notifications_click(void) { return atomic_exchange(&gjClick,0); }
int gj_notifications_error(void) {return atomic_exchange(&gjDispatchError,0);}
void gj_notifications_send(long long intent,const char *title,const char *body) {
    if(!gjCenter){atomic_store(&gjDispatchError,1);return;}
    UNMutableNotificationContent *content=[UNMutableNotificationContent new];
    content.title=[NSString stringWithUTF8String:title];content.body=[NSString stringWithUTF8String:body];
    // No source identifiers in userInfo, no sound, no body/log excerpts.
    UNNotificationRequest *request=[UNNotificationRequest requestWithIdentifier:[NSString stringWithFormat:@"gitjuke-intent-%lld",intent] content:content trigger:nil];
    [gjCenter addNotificationRequest:request withCompletionHandler:^(NSError *error){if(error)atomic_store(&gjDispatchError,1);}];
}
