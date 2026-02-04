/* Wrapper header for Andor SDK3 FFI bindings generation */

#ifdef ANDOR_SDK3_FEATURE

/* Camera API - atcore.h */
#include <atcore.h>

/* Spectrograph API - atspectrograph.h */
#include <atspectrograph.h>

#else

/* Dummy definitions when SDK is not available */
typedef int AT_H;
typedef unsigned short AT_WC;
typedef int AT_BOOL;
typedef long long AT_64;

#define AT_SUCCESS 0
#define AT_HANDLE_UNINITIALISED -1
#define AT_HANDLE_SYSTEM 1
#define AT_TRUE 1
#define AT_FALSE 0

#endif
