// Adapter-only native qualification. No gh, credentials, remote data or production app state.
#import <Cocoa/Cocoa.h>
#import <UserNotifications/UserNotifications.h>
void gj_notifications_initialize(void);void gj_notifications_refresh(void);int gj_notifications_permission(void);void gj_notifications_request(void);void gj_notifications_send(long long,const char*,const char*);long long gj_notifications_click(void);int gj_notifications_error(void);
@interface Qualification : NSObject <NSApplicationDelegate>
@property NSWindow *window;@property NSTextField *status;@property NSString *logPath;
@end
@implementation Qualification
- (void)log:(NSString*)text {NSFileHandle *file=[NSFileHandle fileHandleForWritingAtPath:self.logPath];[file seekToEndOfFile];[file writeData:[[text stringByAppendingString:@"\n"] dataUsingEncoding:NSUTF8StringEncoding]];[file closeFile];}
- (void)applicationDidFinishLaunching:(NSNotification*)note {
 self.logPath=@"/private/tmp/gitjuke-alerts-native-adapter.log";[[NSFileManager defaultManager] createFileAtPath:self.logPath contents:[NSData data] attributes:nil];gj_notifications_initialize();
 self.window=[[NSWindow alloc] initWithContentRect:NSMakeRect(100,100,650,280) styleMask:NSWindowStyleMaskTitled|NSWindowStyleMaskClosable backing:NSBackingStoreBuffered defer:NO];self.window.title=@"GitJuke Alerts Qualification — LOCAL ADAPTER ONLY";
 NSTextField *text=[NSTextField wrappingLabelWithString:@"Isolated local notification adapter. No account or source data. Permission applies only to dev.hmrguez.gitjuke.alerts.qualification. Enable is explicit; Send uses generic synthetic text. Reader routing and durable attempts are tested separately."];text.frame=NSMakeRect(24,140,600,110);[self.window.contentView addSubview:text];
 self.status=[NSTextField labelWithString:@"Permission: checking"];self.status.frame=NSMakeRect(24,100,600,25);[self.window.contentView addSubview:self.status];
 NSButton *enable=[NSButton buttonWithTitle:@"Enable qualification notifications" target:self action:@selector(enable:)];enable.frame=NSMakeRect(24,45,280,35);[self.window.contentView addSubview:enable];
 NSButton *send=[NSButton buttonWithTitle:@"Send generic qualification alert" target:self action:@selector(send:)];send.frame=NSMakeRect(320,45,290,35);[self.window.contentView addSubview:send];
 [NSTimer scheduledTimerWithTimeInterval:1 target:self selector:@selector(tick:) userInfo:nil repeats:YES];[self.window center];[self.window makeKeyAndOrderFront:nil];[NSApp activateIgnoringOtherApps:YES];[self log:@"started; permission never requested automatically"];
}
- (void)enable:(id)sender{[self log:@"explicit enable"];gj_notifications_request();}
- (void)send:(id)sender{if(gj_notifications_permission()!=3){[self log:@"dispatch refused: permission not granted"];return;}gj_notifications_send(4242,"Personal activity","2 new activity items · personal activity");[self log:@"adapter dispatch invoked; synthetic id 4242; no source metadata"];}
- (void)tick:(id)sender{gj_notifications_refresh();int p=gj_notifications_permission();self.status.stringValue=[NSString stringWithFormat:@"Permission: %d (1 not determined, 2 denied, 3 granted)",p];static int last=-1;if(last!=p){[self log:self.status.stringValue];last=p;}long long click=gj_notifications_click();if(click){[self log:[NSString stringWithFormat:@"OS default click received: %lld",click]];[self.window makeKeyAndOrderFront:nil];[NSApp activateIgnoringOtherApps:YES];}if(gj_notifications_error())[self log:@"OS enqueue returned an error; no retry"];}
@end
int main(){@autoreleasepool{NSApplication *app=[NSApplication sharedApplication];Qualification *delegate=[Qualification new];app.delegate=delegate;[app run];}return 0;}
